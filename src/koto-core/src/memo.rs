use crate::hal::Rect;
use crate::layout::TextLayout;

pub const MEMO_DEFAULT_CAPACITY: usize = 4096;
pub const MEMO_MAX_DIRTY_LINES: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoError {
    CapacityExceeded,
    InvalidText,
    DirtyListFull,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoMove {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoDirty {
    None,
    Ime,
    Line(u16),
    Content,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoDirtyLines<const N: usize> {
    lines: [Option<u16>; N],
    len: usize,
    content: bool,
    ime: bool,
}

impl<const N: usize> MemoDirtyLines<N> {
    pub const fn new() -> Self {
        Self {
            lines: [None; N],
            len: 0,
            content: false,
            ime: false,
        }
    }

    pub fn clear(&mut self) {
        self.lines = [None; N];
        self.len = 0;
        self.content = false;
        self.ime = false;
    }

    pub fn mark(&mut self, dirty: MemoDirty) -> Result<(), MemoError> {
        match dirty {
            MemoDirty::None => Ok(()),
            MemoDirty::Ime => {
                self.ime = true;
                Ok(())
            }
            MemoDirty::Content => {
                self.content = true;
                Ok(())
            }
            MemoDirty::Line(line) => {
                if self.content || self.lines[..self.len].contains(&Some(line)) {
                    return Ok(());
                }
                if self.len >= N {
                    self.content = true;
                    return Err(MemoError::DirtyListFull);
                }
                self.lines[self.len] = Some(line);
                self.len += 1;
                Ok(())
            }
        }
    }

    pub fn mark_ime(&mut self) {
        self.ime = true;
    }

    pub fn content_dirty(&self) -> bool {
        self.content
    }

    pub fn ime_dirty(&self) -> bool {
        self.ime
    }

    pub fn lines(&self) -> &[Option<u16>] {
        &self.lines[..self.len]
    }

    pub fn rects<'a>(&'a self, layout: TextLayout) -> MemoDirtyRects<'a, N> {
        MemoDirtyRects {
            dirty: self,
            layout,
            index: 0,
            yielded_content: false,
            yielded_ime: false,
        }
    }
}

impl<const N: usize> Default for MemoDirtyLines<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MemoDirtyRects<'a, const N: usize> {
    dirty: &'a MemoDirtyLines<N>,
    layout: TextLayout,
    index: usize,
    yielded_content: bool,
    yielded_ime: bool,
}

impl<const N: usize> Iterator for MemoDirtyRects<'_, N> {
    type Item = Rect;

    fn next(&mut self) -> Option<Self::Item> {
        if self.dirty.content && !self.yielded_content {
            self.yielded_content = true;
            return Some(self.layout.content);
        }
        if !self.dirty.content && self.index < self.dirty.len {
            let line = self.dirty.lines[self.index]?;
            self.index += 1;
            return self
                .layout
                .content_cell_rect(0, line, self.layout.content_cols);
        }
        if self.dirty.ime && !self.yielded_ime {
            self.yielded_ime = true;
            return Some(self.layout.ime);
        }
        None
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoEditor<const CAP: usize, const DIRTY: usize = MEMO_MAX_DIRTY_LINES> {
    bytes: [u8; CAP],
    len: usize,
    cursor: usize,
    scroll_row: usize,
    /// Horizontal scroll offset in display columns, used only when `wrap` is off.
    hscroll: usize,
    /// Soft-wrap long logical lines onto multiple visual rows. When off, lines are
    /// kept on one row and scrolled horizontally to keep the cursor visible.
    wrap: bool,
    /// Bottom rows of the viewport reserved for an overlay (e.g. the IME
    /// conversion panel). The cursor is kept scrolled above them so the line being
    /// edited never sits underneath the overlay.
    reserved_bottom_rows: usize,
    layout: TextLayout,
    dirty: MemoDirtyLines<DIRTY>,
}

impl<const CAP: usize, const DIRTY: usize> MemoEditor<CAP, DIRTY> {
    pub fn new(layout: TextLayout) -> Self {
        let mut dirty = MemoDirtyLines::new();
        let _ = dirty.mark(MemoDirty::Content);
        dirty.mark_ime();
        Self {
            bytes: [0; CAP],
            len: 0,
            cursor: 0,
            scroll_row: 0,
            hscroll: 0,
            wrap: true,
            reserved_bottom_rows: 0,
            layout,
            dirty,
        }
    }

    /// Reserve the bottom `rows` rows of the viewport for an overlay, clamped so at
    /// least one editable row remains. The cursor is immediately scrolled above the
    /// reserved band if needed.
    pub fn set_reserved_bottom_rows(&mut self, rows: usize) {
        let max_reserved = usize::from(self.layout.content_rows).saturating_sub(1);
        let rows = rows.min(max_reserved);
        if rows == self.reserved_bottom_rows {
            return;
        }
        self.reserved_bottom_rows = rows;
        self.ensure_cursor_visible();
    }

    /// Bottom rows currently reserved for an overlay.
    pub fn reserved_bottom_rows(&self) -> usize {
        self.reserved_bottom_rows
    }

    /// Whether long lines soft-wrap (`true`) or scroll horizontally (`false`).
    pub fn is_wrap(&self) -> bool {
        self.wrap
    }

    /// Horizontal scroll offset in display columns (`0` while wrapping).
    pub fn hscroll(&self) -> usize {
        self.hscroll
    }

    /// Toggle soft wrapping, resetting scroll so the cursor stays in view.
    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        self.hscroll = 0;
        self.scroll_row = 0;
        self.ensure_cursor_visible();
        let _ = self.dirty.mark(MemoDirty::Content);
    }

    /// Display-cell width of the cursor's logical line, for a horizontal scrollbar.
    pub fn cursor_line_cols(&self) -> usize {
        let start = self.line_start(self.cursor);
        let end = self.line_end(start);
        self.as_str()[start..end]
            .chars()
            .map(display_cell_width)
            .sum()
    }

    pub fn capacity(&self) -> usize {
        CAP
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn scroll_row(&self) -> usize {
        self.scroll_row
    }

    pub fn total_logical_lines(&self) -> usize {
        self.bytes[..self.len]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            + 1
    }

    /// The cursor's visual (wrapped) row relative to the scroll position, or
    /// `None` when it is scrolled out of the viewport.
    pub fn cursor_visible_row(&self) -> Option<u16> {
        let (vrow, _) = self.cursor_visual();
        vrow.checked_sub(self.scroll_row)
            .and_then(|row| u16::try_from(row).ok())
            .filter(|row| *row < self.layout.content_rows)
    }

    /// The cursor's column within its logical line, in display cells.
    pub fn cursor_column(&self) -> usize {
        self.column_in_line(self.cursor)
    }

    /// The cursor's column within its visual row, for caret placement. With wrap
    /// off this is the logical column shifted by the horizontal scroll.
    pub fn cursor_display_col(&self) -> usize {
        if self.wrap {
            self.cursor_visual().1
        } else {
            self.column_in_line(self.cursor)
                .saturating_sub(self.hscroll)
        }
    }

    /// The cursor's zero-based logical line index (independent of wrapping), for a
    /// document `Ln N` status.
    pub fn cursor_logical_line(&self) -> usize {
        self.line_index_at(self.cursor)
    }

    /// Total visual (wrapped) rows in the document, for vertical scroll indicators.
    pub fn total_visual_rows(&self) -> usize {
        let mut total = 0;
        let mut line_start = 0;
        loop {
            let end = self.line_end(line_start);
            total += self.line_visual_rows(line_start, end);
            if end >= self.len {
                break;
            }
            line_start = end + 1;
        }
        total
    }

    pub fn cursor_rect(&self) -> Option<Rect> {
        let col = u16::try_from(self.cursor_display_col()).ok()?;
        let row = self.cursor_visible_row()?;
        self.layout.content_cell_rect(col, row, 1)
    }

    pub fn layout(&self) -> TextLayout {
        self.layout
    }

    pub fn dirty(&self) -> &MemoDirtyLines<DIRTY> {
        &self.dirty
    }

    pub fn dirty_mut(&mut self) -> &mut MemoDirtyLines<DIRTY> {
        &mut self.dirty
    }

    pub fn take_dirty(&mut self) -> MemoDirtyLines<DIRTY> {
        let dirty = self.dirty.clone();
        self.dirty.clear();
        dirty
    }

    pub fn load_str(&mut self, text: &str) -> Result<(), MemoError> {
        if text.len() > CAP {
            return Err(MemoError::CapacityExceeded);
        }
        self.bytes[..text.len()].copy_from_slice(text.as_bytes());
        self.len = text.len();
        self.cursor = self.len;
        self.scroll_row = 0;
        self.ensure_cursor_visible();
        let _ = self.dirty.mark(MemoDirty::Content);
        Ok(())
    }

    pub fn insert_char(&mut self, ch: char) -> Result<(), MemoError> {
        let mut buf = [0u8; 4];
        let text = ch.encode_utf8(&mut buf);
        self.insert_str(text)
    }

    pub fn insert_str(&mut self, text: &str) -> Result<(), MemoError> {
        if self.len + text.len() > CAP {
            return Err(MemoError::CapacityExceeded);
        }
        if !self.cursor_is_boundary() {
            return Err(MemoError::InvalidText);
        }
        let line = self.cursor_visible_line();
        self.bytes
            .copy_within(self.cursor..self.len, self.cursor + text.len());
        self.bytes[self.cursor..self.cursor + text.len()].copy_from_slice(text.as_bytes());
        self.cursor += text.len();
        self.len += text.len();
        self.mark_edit(line, text.contains('\n'));
        self.ensure_cursor_visible();
        Ok(())
    }

    pub fn backspace(&mut self) -> Result<bool, MemoError> {
        if self.cursor == 0 {
            return Ok(false);
        }
        let line = self.cursor_visible_line();
        let prev = self
            .prev_boundary(self.cursor)
            .ok_or(MemoError::InvalidText)?;
        let removed_newline = self.bytes[prev..self.cursor].contains(&b'\n');
        let removed = self.cursor - prev;
        self.bytes.copy_within(self.cursor..self.len, prev);
        self.cursor = prev;
        self.len -= removed;
        self.mark_edit(line, removed_newline);
        self.ensure_cursor_visible();
        Ok(true)
    }

    pub fn delete(&mut self) -> Result<bool, MemoError> {
        if self.cursor >= self.len {
            return Ok(false);
        }
        let line = self.cursor_visible_line();
        let next = self
            .next_boundary(self.cursor)
            .ok_or(MemoError::InvalidText)?;
        let removed_newline = self.bytes[self.cursor..next].contains(&b'\n');
        let removed = next - self.cursor;
        self.bytes.copy_within(next..self.len, self.cursor);
        self.len -= removed;
        self.mark_edit(line, removed_newline);
        self.ensure_cursor_visible();
        Ok(true)
    }

    pub fn move_cursor(&mut self, movement: MemoMove) {
        match movement {
            MemoMove::Left => {
                if let Some(prev) = self.prev_boundary(self.cursor) {
                    self.cursor = prev;
                }
            }
            MemoMove::Right => {
                if let Some(next) = self.next_boundary(self.cursor) {
                    self.cursor = next;
                }
            }
            MemoMove::Home => self.cursor = self.line_start(self.cursor),
            MemoMove::End => self.cursor = self.line_end(self.cursor),
            MemoMove::Up => self.move_vertical(-1),
            MemoMove::Down => self.move_vertical(1),
        }
        self.ensure_cursor_visible();
    }

    pub fn visible_line(&self, row: u16) -> Option<&str> {
        if row >= self.layout.content_rows {
            return None;
        }
        let (start, end) = self.visual_segment(self.scroll_row + usize::from(row))?;
        if self.wrap {
            return core::str::from_utf8(&self.bytes[start..end]).ok();
        }
        // Wrap off: window the logical line to the horizontally scrolled columns.
        let cols = self.content_columns();
        let win_start = self.offset_at_chunk_col(start, end, self.hscroll);
        let win_end = self.offset_at_chunk_col(start, end, self.hscroll + cols);
        core::str::from_utf8(&self.bytes[win_start..win_end]).ok()
    }

    fn mark_edit(&mut self, line: Option<u16>, whole_content: bool) {
        if whole_content {
            let _ = self.dirty.mark(MemoDirty::Content);
        } else if let Some(line) = line {
            let _ = self.dirty.mark(MemoDirty::Line(line));
        } else {
            let _ = self.dirty.mark(MemoDirty::Content);
        }
    }

    fn cursor_is_boundary(&self) -> bool {
        self.as_str().is_char_boundary(self.cursor)
    }

    fn cursor_visible_line(&self) -> Option<u16> {
        let absolute = self.line_index_at(self.cursor);
        absolute
            .checked_sub(self.scroll_row)
            .and_then(|line| u16::try_from(line).ok())
            .filter(|line| *line < self.layout.content_rows)
    }

    fn ensure_cursor_visible(&mut self) {
        let cursor_row = self.cursor_visual().0;
        // Keep the cursor inside the rows not covered by a reserved overlay band,
        // leaving at least one editable row.
        let rows = usize::from(self.layout.content_rows)
            .saturating_sub(self.reserved_bottom_rows)
            .max(1);
        if cursor_row < self.scroll_row {
            self.scroll_row = cursor_row;
            let _ = self.dirty.mark(MemoDirty::Content);
        } else if rows > 0 && cursor_row >= self.scroll_row + rows {
            self.scroll_row = cursor_row + 1 - rows;
            let _ = self.dirty.mark(MemoDirty::Content);
        }
        if self.wrap {
            self.hscroll = 0;
            return;
        }
        // Wrap off: keep the cursor column within the horizontally scrolled window.
        let cols = self.content_columns();
        let column = self.column_in_line(self.cursor);
        if column < self.hscroll {
            self.hscroll = column;
            let _ = self.dirty.mark(MemoDirty::Content);
        } else if cols > 0 && column >= self.hscroll + cols {
            self.hscroll = column + 1 - cols;
            let _ = self.dirty.mark(MemoDirty::Content);
        }
    }

    fn move_vertical(&mut self, delta: i32) {
        // Move by one visual (wrapped) row, keeping the display column.
        let (vrow, col) = self.cursor_visual();
        let target = if delta < 0 {
            vrow.saturating_sub(1)
        } else {
            vrow.saturating_add(1)
        };
        if let Some((start, end)) = self.visual_segment(target) {
            self.cursor = self.offset_at_chunk_col(start, end, col);
        }
    }

    /// The content width in display cells (columns) a visual row may hold.
    fn content_columns(&self) -> usize {
        let cols = usize::from(self.layout.content_cols);
        if cols == 0 {
            1
        } else {
            cols
        }
    }

    /// Split a logical line `[start, end)` into visual rows of at most
    /// `content_columns()` display cells, invoking `f(seg_start, seg_end)` for each
    /// (always at least once, so empty lines yield one empty visual row).
    fn for_each_chunk(&self, start: usize, end: usize, mut f: impl FnMut(usize, usize)) {
        if !self.wrap {
            // One visual row per logical line; horizontal scroll handles overflow.
            f(start, end);
            return;
        }
        let cols = self.content_columns();
        let text = self.as_str();
        let mut seg_start = start;
        let mut width = 0usize;
        for (rel, ch) in text[start..end].char_indices() {
            let abs = start + rel;
            let cw = display_cell_width(ch);
            if width + cw > cols && abs > seg_start {
                f(seg_start, abs);
                seg_start = abs;
                width = cw;
            } else {
                width += cw;
            }
        }
        f(seg_start, end);
    }

    fn line_visual_rows(&self, start: usize, end: usize) -> usize {
        let mut rows = 0;
        self.for_each_chunk(start, end, |_, _| rows += 1);
        rows
    }

    /// Byte range of the `vrow`-th visual row across the whole document.
    fn visual_segment(&self, vrow: usize) -> Option<(usize, usize)> {
        let mut remaining = vrow;
        let mut found = None;
        let mut line_start = 0;
        loop {
            let end = self.line_end(line_start);
            self.for_each_chunk(line_start, end, |start, stop| {
                if found.is_none() {
                    if remaining == 0 {
                        found = Some((start, stop));
                    } else {
                        remaining -= 1;
                    }
                }
            });
            if found.is_some() {
                return found;
            }
            if end >= self.len {
                return None;
            }
            line_start = end + 1;
        }
    }

    /// The cursor's `(visual_row, column_within_row)`.
    fn cursor_visual(&self) -> (usize, usize) {
        if !self.wrap {
            return (
                self.line_index_at(self.cursor),
                self.column_in_line(self.cursor),
            );
        }
        let cursor = self.cursor.min(self.len);
        let cols = self.content_columns();
        let text = self.as_str();
        let mut vrow = 0usize;
        let mut line_start = 0;
        loop {
            let end = self.line_end(line_start);
            if cursor >= line_start && cursor <= end {
                let mut local = 0usize;
                let mut seg_start = line_start;
                let mut width = 0usize;
                let mut col = 0usize;
                for (rel, ch) in text[line_start..end].char_indices() {
                    let abs = line_start + rel;
                    let cw = display_cell_width(ch);
                    // Detect wrap before checking cursor so that a cursor at the
                    // first character of a new visual row gets col=0, not the
                    // accumulated column of the previous row.
                    if width + cw > cols && abs > seg_start {
                        local += 1;
                        seg_start = abs;
                        width = 0;
                        col = 0;
                    }
                    if abs == cursor {
                        return (vrow + local, col);
                    }
                    width += cw;
                    col += cw;
                }
                return (vrow + local, col);
            }
            vrow += self.line_visual_rows(line_start, end);
            if end >= self.len {
                break;
            }
            line_start = end + 1;
        }
        (vrow, 0)
    }

    /// The byte offset at display `column` within a visual row `[start, end)`.
    fn offset_at_chunk_col(&self, start: usize, end: usize, column: usize) -> usize {
        let text = self.as_str();
        let mut offset = start;
        let mut cells = 0usize;
        for ch in text[start..end].chars() {
            let cw = display_cell_width(ch);
            if cells + cw > column {
                break;
            }
            cells += cw;
            offset += ch.len_utf8();
        }
        offset
    }

    fn line_index_at(&self, offset: usize) -> usize {
        self.bytes[..offset.min(self.len)]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
    }

    fn column_in_line(&self, offset: usize) -> usize {
        let line_start = self.line_start(offset);
        let text = self.as_str();
        text[line_start..offset.min(self.len)]
            .chars()
            .map(display_cell_width)
            .sum()
    }

    fn line_start(&self, offset: usize) -> usize {
        let offset = offset.min(self.len);
        self.bytes[..offset]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|pos| pos + 1)
            .unwrap_or(0)
    }

    fn line_end(&self, offset: usize) -> usize {
        let offset = offset.min(self.len);
        self.bytes[offset..self.len]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|pos| offset + pos)
            .unwrap_or(self.len)
    }

    fn prev_boundary(&self, offset: usize) -> Option<usize> {
        if offset == 0 || offset > self.len {
            return None;
        }
        let text = self.as_str();
        let mut prev = 0usize;
        for (index, _) in text.char_indices() {
            if index >= offset {
                break;
            }
            prev = index;
        }
        Some(prev)
    }

    fn next_boundary(&self, offset: usize) -> Option<usize> {
        if offset >= self.len || !self.as_str().is_char_boundary(offset) {
            return None;
        }
        let text = self.as_str();
        text[offset..]
            .chars()
            .next()
            .map(|ch| offset + ch.len_utf8())
    }
}

fn display_cell_width(ch: char) -> usize {
    if ch.is_ascii() {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hal::PixelFormat;
    use crate::layout::CellMetrics;
    use crate::render::RenderSurface;

    fn layout() -> TextLayout {
        TextLayout::new(
            RenderSurface::new(320, 320, PixelFormat::Rgb565),
            CellMetrics::FONT_8X12,
            2,
        )
        .unwrap()
    }

    #[test]
    fn inserts_text_with_fixed_capacity() {
        let mut editor = MemoEditor::<8>::new(layout());

        editor.insert_str("memo").unwrap();
        assert_eq!(editor.as_str(), "memo");
        assert_eq!(editor.cursor(), 4);
        assert_eq!(
            editor.insert_str(" too big"),
            Err(MemoError::CapacityExceeded)
        );
    }

    #[test]
    fn moves_cursor_and_edits_utf8_text() {
        let mut editor = MemoEditor::<32>::new(layout());
        editor.insert_str("あい").unwrap();
        editor.move_cursor(MemoMove::Left);
        editor.insert_char('う').unwrap();

        assert_eq!(editor.as_str(), "あうい");
        assert!(editor.backspace().unwrap());
        assert_eq!(editor.as_str(), "あい");
        assert!(editor.delete().unwrap());
        assert_eq!(editor.as_str(), "あ");
    }

    #[test]
    fn handles_newlines_home_end_and_visible_lines() {
        let mut editor = MemoEditor::<64>::new(layout());
        editor.insert_str("one\ntwo\nthree").unwrap();

        editor.move_cursor(MemoMove::Home);
        assert_eq!(editor.cursor(), 8); // start of "three"
        editor.move_cursor(MemoMove::End);
        assert_eq!(editor.cursor(), editor.as_str().len());

        assert_eq!(editor.visible_line(0), Some("one"));
        assert_eq!(editor.visible_line(1), Some("two"));
        assert_eq!(editor.visible_line(2), Some("three"));
    }

    #[test]
    fn vertical_movement_preserves_column_where_possible() {
        let mut editor = MemoEditor::<64>::new(layout());
        editor.insert_str("abcd\nef\nghij").unwrap();
        editor.move_cursor(MemoMove::Home);
        editor.move_cursor(MemoMove::Right);
        editor.move_cursor(MemoMove::Up);

        assert_eq!(&editor.as_str()[editor.cursor()..editor.cursor() + 1], "f");

        editor.move_cursor(MemoMove::Down);
        editor.move_cursor(MemoMove::Down);
        assert_eq!(&editor.as_str()[editor.cursor()..editor.cursor() + 1], "h");
    }

    #[test]
    fn cursor_column_uses_display_cells_for_mixed_width_text() {
        let mut editor = MemoEditor::<64>::new(layout());
        editor.insert_str("aあb").unwrap();

        assert_eq!(editor.cursor_column(), 4);
        editor.move_cursor(MemoMove::Left);
        assert_eq!(editor.cursor_column(), 3);
        editor.move_cursor(MemoMove::Left);
        assert_eq!(editor.cursor_column(), 1);
    }

    #[test]
    fn vertical_movement_preserves_display_cell_column() {
        let mut editor = MemoEditor::<64>::new(layout());
        editor.insert_str("aあb\nabcd").unwrap();

        editor.move_cursor(MemoMove::Home);
        editor.move_cursor(MemoMove::Right);
        editor.move_cursor(MemoMove::Right);
        assert_eq!(editor.cursor_column(), 2);
        editor.move_cursor(MemoMove::Down);

        assert_eq!(editor.cursor_column(), 2);
        assert_eq!(&editor.as_str()[editor.cursor()..editor.cursor() + 1], "c");
    }

    #[test]
    fn cursor_rect_uses_layout_cell_metrics_for_mixed_width_text() {
        let custom = TextLayout::new(
            RenderSurface::new(120, 80, PixelFormat::Rgb565),
            CellMetrics {
                cell_width: 6,
                cell_height: 13,
            },
            1,
        )
        .unwrap();
        let mut editor = MemoEditor::<64>::new(custom);
        editor.insert_str("Aあ字\nabcdef").unwrap();
        editor.move_cursor(MemoMove::Left);

        assert_eq!(
            editor.cursor_rect(),
            Some(Rect {
                x: 6 * 5,
                y: 26,
                w: 6,
                h: 13
            })
        );

        editor.move_cursor(MemoMove::Up);
        assert_eq!(
            editor.cursor_rect(),
            Some(Rect {
                x: 6 * 5,
                y: 13,
                w: 6,
                h: 13
            })
        );
    }

    #[test]
    fn scrolls_to_keep_cursor_visible() {
        let small = TextLayout::new(
            RenderSurface::new(80, 60, PixelFormat::Rgb565),
            CellMetrics::FONT_8X12,
            1,
        )
        .unwrap();
        let mut editor = MemoEditor::<128>::new(small);
        editor.insert_str("0\n1\n2\n3\n4").unwrap();

        assert!(editor.scroll_row() > 0);
        assert_eq!(editor.visible_line(0), Some("2"));
    }

    #[test]
    fn reserved_rows_scroll_cursor_above_overlay() {
        // 80x60 / 8x12 / 1 IME line -> content_rows 3.
        let small = TextLayout::new(
            RenderSurface::new(80, 60, PixelFormat::Rgb565),
            CellMetrics::FONT_8X12,
            1,
        )
        .unwrap();
        let mut editor = MemoEditor::<128>::new(small);
        editor.insert_str("0\n1\n2\n3\n4").unwrap();

        // With 3 visible rows the caret on the last line sits on the bottom row.
        assert_eq!(editor.cursor_visible_row(), Some(2));

        // Reserve the bottom row for an overlay: the caret scrolls up out of it.
        editor.set_reserved_bottom_rows(1);
        assert_eq!(editor.reserved_bottom_rows(), 1);
        assert_eq!(editor.cursor_visible_row(), Some(1));
        assert_eq!(editor.visible_line(0), Some("3"));

        // Clearing the reservation keeps the caret visible (no forced scroll back).
        editor.set_reserved_bottom_rows(0);
        assert_eq!(editor.reserved_bottom_rows(), 0);
        assert!(editor.cursor_visible_row().is_some());

        // Reserving more than the viewport clamps to leave one editable row.
        editor.set_reserved_bottom_rows(99);
        assert_eq!(editor.reserved_bottom_rows(), 2);
        assert_eq!(editor.cursor_visible_row(), Some(0));
    }

    #[test]
    fn wrap_splits_long_lines_into_visual_rows() {
        // 80x60 / 8x12 / 1 IME line -> content_cols 10, content_rows 3.
        let small = TextLayout::new(
            RenderSurface::new(80, 60, PixelFormat::Rgb565),
            CellMetrics::FONT_8X12,
            1,
        )
        .unwrap();
        let mut editor = MemoEditor::<128>::new(small);
        editor.insert_str("abcdefghijklmno").unwrap(); // 15 columns

        assert!(editor.is_wrap());
        assert_eq!(editor.total_visual_rows(), 2);
        assert_eq!(editor.visible_line(0), Some("abcdefghij"));
        assert_eq!(editor.visible_line(1), Some("klmno"));
        // Caret on the second visual row at column 5 (15 - 10).
        assert_eq!(editor.cursor_visible_row(), Some(1));
        assert_eq!(editor.cursor_display_col(), 5);
    }

    #[test]
    fn cursor_at_wrap_boundary_starts_new_visual_row_at_col_zero() {
        // 80x60 / 8x12 / 1 IME line -> content_cols 10, content_rows 3.
        let small = TextLayout::new(
            RenderSurface::new(80, 60, PixelFormat::Rgb565),
            CellMetrics::FONT_8X12,
            1,
        )
        .unwrap();
        let mut editor = MemoEditor::<128>::new(small);
        // 11 chars wrap at column 10. After insert cursor is at offset 11 (row 1, col 1).
        // Move left once to reach offset 10 — the first char of the second visual row.
        editor.insert_str("abcdefghijk").unwrap();
        editor.move_cursor(MemoMove::Left);
        assert_eq!(editor.cursor(), 10);
        assert_eq!(editor.cursor_display_col(), 0);
        assert_eq!(editor.cursor_visible_row(), Some(1));
    }

    #[test]
    fn no_wrap_mode_scrolls_horizontally() {
        let small = TextLayout::new(
            RenderSurface::new(80, 60, PixelFormat::Rgb565),
            CellMetrics::FONT_8X12,
            1,
        )
        .unwrap();
        let mut editor = MemoEditor::<128>::new(small);
        editor.insert_str("abcdefghijklmno").unwrap(); // 15 columns

        editor.toggle_wrap();
        assert!(!editor.is_wrap());
        // One visual row; the line is scrolled to keep the caret (col 15) in view.
        assert_eq!(editor.total_visual_rows(), 1);
        assert_eq!(editor.hscroll(), 6); // 15 + 1 - 10
        assert_eq!(editor.cursor_line_cols(), 15);
        assert_eq!(editor.visible_line(0), Some("ghijklmno"));
        assert_eq!(editor.cursor_display_col(), 9); // 15 - 6
        assert_eq!(editor.cursor_visible_row(), Some(0));

        // Toggling back restores wrapping and clears horizontal scroll.
        editor.toggle_wrap();
        assert!(editor.is_wrap());
        assert_eq!(editor.hscroll(), 0);
        assert_eq!(editor.visible_line(0), Some("abcdefghij"));
    }

    #[test]
    fn reports_total_logical_lines_for_scrollbars() {
        let mut editor = MemoEditor::<64>::new(layout());
        assert_eq!(editor.total_logical_lines(), 1);

        editor.insert_str("one\ntwo\nthree\n").unwrap();
        assert_eq!(editor.total_logical_lines(), 4);
    }

    #[test]
    fn reports_dirty_lines_and_ime_rects() {
        let mut editor = MemoEditor::<64>::new(layout());
        editor.take_dirty();

        editor.insert_str("abc").unwrap();
        let dirty = editor.take_dirty();
        assert!(!dirty.content_dirty());
        assert_eq!(dirty.lines(), &[Some(0)]);
        let rects: std::vec::Vec<Rect> = dirty.rects(layout()).collect();
        assert_eq!(
            rects,
            [Rect {
                x: 0,
                y: 12,
                w: 320,
                h: 12
            }]
        );

        let mut dirty = MemoDirtyLines::<2>::new();
        dirty.mark_ime();
        let rects: std::vec::Vec<Rect> = dirty.rects(layout()).collect();
        assert_eq!(rects, [layout().ime]);
    }

    #[test]
    fn newline_edit_marks_content_dirty() {
        let mut editor = MemoEditor::<64>::new(layout());
        editor.take_dirty();

        editor.insert_str("a\nb").unwrap();

        assert!(editor.dirty().content_dirty());
    }
}
