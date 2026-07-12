use core::cmp::Ordering;

use crate::font::BitmapFont;
use crate::hal::{InputState, PowerState};
use crate::package::{PackageIconTheme, PackageInfo, PackageList, MAX_CATEGORY_LEN, MAX_PACKAGES};
use crate::raster::{Canvas, Rgb565};
use crate::render::{RenderCommand, RenderCommandList, RenderError, RenderSurface};
use crate::KOTO_COPYRIGHT_NOTICE;

/// Title shown in the shell header band.
pub const SHELL_TITLE: &str = "ホーム";
/// Width in pixels of each page-indicator triangle.
pub const SHELL_PAGE_ARROW_WIDTH: i32 = 6;
/// Left padding for header and row text, in pixels.
pub const SHELL_TEXT_PADDING: i32 = 4;
pub const SHELL_GRID_COLUMNS: usize = 4;
pub const SHELL_LOW_BATTERY_PERCENT: u8 = 15;
pub const SHELL_LOW_BATTERY_MILLIVOLTS: u16 = 3400;
pub const SHELL_STATUS_TEXT_CAPACITY: usize = 16;
pub const SHELL_DETAIL_TEXT_CAPACITY: usize = 128;
pub const SHELL_SELECTION_FEEDBACK_FRAMES: u8 = 6;
pub const SHELL_PAGE_FEEDBACK_FRAMES: u8 = 8;
pub const SHELL_PANE_TRANSITION_FRAMES: u8 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellSound {
    Navigation,
    Confirm,
    Cancel,
}

/// Colors used when painting the shell to a [`Canvas`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellPalette {
    pub background: Rgb565,
    pub header_bg: Rgb565,
    pub header_fg: Rgb565,
    pub row_bg: Rgb565,
    pub row_fg: Rgb565,
    pub selected_bg: Rgb565,
    pub selected_fg: Rgb565,
    pub selected_border: Rgb565,
    pub icon_bg: Rgb565,
    pub icon_fg: Rgb565,
    pub icon_shadow: Rgb565,
    pub pane_bg: Rgb565,
    pub pane_fg: Rgb565,
    pub pane_accent: Rgb565,
    pub separator: Rgb565,
    pub status_strip_bg: Rgb565,
    pub status_strip_fg: Rgb565,
    pub command_bar_bg: Rgb565,
    pub command_bar_fg: Rgb565,
    pub command_key_bg: Rgb565,
    pub status_ok: Rgb565,
    pub status_warn: Rgb565,
    pub status_dim: Rgb565,
}

impl ShellPalette {
    /// Light "PDA home screen" theme matching the Koto Memo light shell.
    pub const DEFAULT: ShellPalette = ShellPalette {
        background: Rgb565::from_rgb8(244, 244, 238),
        header_bg: Rgb565::from_rgb8(26, 34, 52),
        header_fg: Rgb565::from_rgb8(236, 240, 248),
        row_bg: Rgb565::from_rgb8(244, 244, 238),
        row_fg: Rgb565::from_rgb8(40, 44, 52),
        selected_bg: Rgb565::from_rgb8(255, 255, 255),
        selected_fg: Rgb565::from_rgb8(28, 58, 110),
        selected_border: Rgb565::from_rgb8(40, 96, 176),
        icon_bg: Rgb565::from_rgb8(232, 232, 224),
        icon_fg: Rgb565::from_rgb8(56, 64, 72),
        icon_shadow: Rgb565::from_rgb8(150, 156, 150),
        pane_bg: Rgb565::from_rgb8(255, 255, 255),
        pane_fg: Rgb565::from_rgb8(40, 44, 52),
        pane_accent: Rgb565::from_rgb8(36, 86, 166),
        separator: Rgb565::from_rgb8(176, 180, 176),
        status_strip_bg: Rgb565::from_rgb8(228, 230, 232),
        status_strip_fg: Rgb565::from_rgb8(40, 44, 52),
        command_bar_bg: Rgb565::from_rgb8(26, 34, 52),
        command_bar_fg: Rgb565::from_rgb8(236, 240, 248),
        command_key_bg: Rgb565::from_rgb8(60, 70, 92),
        status_ok: Rgb565::from_rgb8(96, 200, 120),
        status_warn: Rgb565::from_rgb8(224, 96, 84),
        status_dim: Rgb565::from_rgb8(150, 160, 180),
    };
}

impl Default for ShellPalette {
    fn default() -> Self {
        ShellPalette::DEFAULT
    }
}

pub const SHELL_SURFACE: RenderSurface =
    RenderSurface::new(320, 320, crate::hal::PixelFormat::Rgb565);
pub const SHELL_HEADER_HEIGHT: i32 = 20;
/// Secondary status strip directly above the command bar (free memory, selection).
pub const SHELL_STATUS_STRIP_HEIGHT: i32 = 14;
/// Bottom command bar listing available shell actions.
pub const SHELL_COMMAND_BAR_HEIGHT: i32 = 18;
pub const SHELL_FOOTER_HEIGHT: i32 = SHELL_STATUS_STRIP_HEIGHT + SHELL_COMMAND_BAR_HEIGHT;
/// Width of the right-hand details pane when it is shown.
pub const SHELL_PANE_WIDTH: i32 = 124;
pub const SHELL_TILE_HEIGHT: i32 = 84;
pub const SHELL_ICON_SIZE: i32 = 40;

/// Grid columns when the details pane is hidden (full-width launcher).
pub const SHELL_GRID_COLUMNS_NO_PANE: usize = SHELL_GRID_COLUMNS;
/// Grid columns when the details pane is shown (narrower launcher).
pub const SHELL_GRID_COLUMNS_WITH_PANE: usize = 3;

/// Vertical space available to the launcher grid between header and footer.
pub const SHELL_GRID_AREA_HEIGHT: i32 =
    SHELL_SURFACE.height as i32 - SHELL_HEADER_HEIGHT - SHELL_FOOTER_HEIGHT;
/// Number of icon rows that fit in the launcher grid.
pub const SHELL_GRID_ROWS: usize = (SHELL_GRID_AREA_HEIGHT / SHELL_TILE_HEIGHT) as usize;

/// Maximum number of icons visible on one page across both layout modes
/// (the pane-hidden grid has the most columns).
pub const SHELL_VISIBLE_ITEMS: usize = SHELL_GRID_ROWS * SHELL_GRID_COLUMNS_NO_PANE;

/// Upper bound on render commands emitted by [`ShellState::render_list`]:
/// header, page indicator, status strip, command bar, details pane, and one per
/// visible tile. The pane-hidden mode has the most tiles but no pane command, so
/// the worst case is `SHELL_VISIBLE_ITEMS` tiles plus four chrome commands.
pub const SHELL_LIST_COMMANDS: usize = SHELL_VISIBLE_ITEMS + 4;

/// Grid columns for the current layout mode.
pub const fn shell_grid_columns(pane_visible: bool) -> usize {
    if pane_visible {
        SHELL_GRID_COLUMNS_WITH_PANE
    } else {
        SHELL_GRID_COLUMNS_NO_PANE
    }
}

/// Icons visible on one page for the current layout mode.
pub const fn shell_visible_items(pane_visible: bool) -> usize {
    SHELL_GRID_ROWS * shell_grid_columns(pane_visible)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellItem {
    pub package: PackageInfo,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellAction {
    None,
    Launch(PackageInfo),
}

/// One entry in the bottom command bar: a key chip, an action label, an optional
/// state suffix (e.g. `ON`/`OFF`), and whether the action is currently available.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellCommand {
    pub key: &'static str,
    pub label: &'static str,
    pub state: Option<&'static str>,
    pub enabled: bool,
}

/// Number of entries in the shell command bar.
pub const SHELL_COMMAND_COUNT: usize = 6;

/// Launcher sort order, cycled by the sort command.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SortMode {
    /// Manifest discovery order.
    #[default]
    Default,
    /// Alphabetical by display name.
    Name,
    /// Favorites first, then by name.
    Favorite,
}

impl SortMode {
    pub fn label(self) -> &'static str {
        match self {
            SortMode::Default => "既定",
            SortMode::Name => "名前",
            SortMode::Favorite => "★優先",
        }
    }

    fn next(self) -> SortMode {
        match self {
            SortMode::Default => SortMode::Name,
            SortMode::Name => SortMode::Favorite,
            SortMode::Favorite => SortMode::Default,
        }
    }
}

/// A wall-clock timestamp shown in the header. Injected by the host so tests and
/// the simulator can use a fixed value (FR-SHELL-5 / KOTO-0084).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellClock {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

/// Removable storage (SD card) availability for the header indicator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StorageStatus {
    #[default]
    Unknown,
    Present,
    Absent,
}

/// Save / persistence health for the header indicator.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SaveStatus {
    #[default]
    Unknown,
    Saved,
    Unsaved,
}

/// Memory-status snapshot for the shell system view (KOTO-0182).
///
/// All counts are bytes. `Option` fields render as a `----` placeholder when the
/// host has not measured them yet (the app-heap figures before the first app
/// run, or the core-1 audio stack before audio starts), matching the shell's
/// existing clock/battery fallback convention. The device fills these from the
/// KOTO-0170 stack canary, the audio backend, and the PSRAM constants; the
/// simulator injects a fixed representative snapshot. `free_min` mirrors the
/// `phase=176 stack-peak free_min` UART line by construction (both read the same
/// canary), which is how the KOTO-0182 cross-check is satisfied.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoryStatus {
    /// Total on-chip SRAM (RP2040 = 264 KiB).
    pub sram_total: usize,
    /// Bytes occupied by statics (`.data`/`.bss`/`.uninit`), if known.
    pub sram_static_used: Option<usize>,
    /// Minimum free bytes ever left between the statics and the main stack (the
    /// `phase=176` `free_min`). The headline number.
    pub sram_free_min: Option<usize>,
    /// Peak core-0 main-stack depth (the `phase=176` `used`).
    pub stack_peak_used: Option<usize>,
    /// Minimum free bytes left in the core-1 audio worker stack, if measured.
    pub core1_stack_free_min: Option<usize>,
    /// Total resident app-heap span, if an app has been loaded.
    pub app_heap_total: Option<usize>,
    /// App-heap bytes used by the most recent run, if known.
    pub app_heap_last_used: Option<usize>,
    /// External PSRAM total in bytes (0 when absent).
    pub psram_total: usize,
    /// Whether external PSRAM was detected.
    pub psram_present: bool,
    /// Number of code-window slots the PSRAM pager keeps resident.
    pub code_window_slots: u8,
}

impl MemoryStatus {
    /// All-unknown snapshot (const so [`ShellState::empty`] can build it).
    pub const fn unknown() -> Self {
        Self {
            sram_total: 0,
            sram_static_used: None,
            sram_free_min: None,
            stack_peak_used: None,
            core1_stack_free_min: None,
            app_heap_total: None,
            app_heap_last_used: None,
            psram_total: 0,
            psram_present: false,
            code_window_slots: 0,
        }
    }

    /// Free SRAM headroom is a caution below this margin (KOTO-0170 treats
    /// `free_min` under ~4 KiB as stop-ship; 8 KiB gives a visible warning band).
    pub const FREE_MIN_CAUTION_BYTES: usize = 8 * 1024;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellState {
    packages: PackageList,
    /// View order: positions hold indices into `packages`, after category
    /// filtering and sorting. `selected` indexes into this view.
    order: [usize; MAX_PACKAGES],
    order_len: usize,
    selected: usize,
    sort_mode: SortMode,
    category_filter: [u8; MAX_CATEGORY_LEN],
    category_filter_len: usize,
    power_state: PowerState,
    detail_pane: bool,
    /// When set, the launcher is replaced by the system/memory status overlay
    /// (KOTO-0182).
    system_view: bool,
    memory: MemoryStatus,
    clock: Option<ShellClock>,
    storage: StorageStatus,
    save_status: SaveStatus,
    selection_feedback_frames: u8,
    selection_animation_from: usize,
    page_feedback_frames: u8,
    page_animation_from: usize,
    pane_transition_frames: u8,
    pending_sound: Option<ShellSound>,
}

impl ShellState {
    pub fn new(packages: PackageList) -> Self {
        let mut shell = Self::empty();
        shell.packages = packages;
        shell.rebuild_order();
        shell
    }

    /// Const-constructible shell with an empty catalog (KOTO-0172).
    ///
    /// This exists so the device firmware can hold the ~28 KiB `ShellState` in
    /// a const-initialized static instead of building it (and the
    /// `PackageList` moved into `new`) as a stack value — on the RP2040 that
    /// stack is the embassy main task's poll frame, which is reserved on
    /// *every* poll and was the largest term of the measured boot stack peak.
    /// An empty catalog needs no `rebuild_order`; load the real one in place
    /// with [`Self::reload_packages`].
    pub const fn empty() -> Self {
        Self {
            packages: PackageList::new(),
            order: [0; MAX_PACKAGES],
            order_len: 0,
            selected: 0,
            sort_mode: SortMode::Default,
            category_filter: [0; MAX_CATEGORY_LEN],
            category_filter_len: 0,
            power_state: PowerState::unsupported(),
            detail_pane: true,
            system_view: false,
            memory: MemoryStatus::unknown(),
            clock: None,
            storage: StorageStatus::Unknown,
            save_status: SaveStatus::Unknown,
            selection_feedback_frames: 0,
            selection_animation_from: 0,
            page_feedback_frames: 0,
            page_animation_from: 0,
            pane_transition_frames: 0,
            pending_sound: None,
        }
    }

    /// Refill the catalog in place and rebuild the view (KOTO-0172).
    ///
    /// The closure fills the (cleared) list directly inside `self`, so no
    /// `PackageList`-sized temporary ever crosses the caller's stack; its
    /// return value passes through (the device loader reports its
    /// `StorageStatus` this way). Selection resets with the new catalog; the
    /// sort mode and category filter are honored by the rebuilt view like any
    /// other catalog change.
    pub fn reload_packages<R>(&mut self, fill: impl FnOnce(&mut PackageList) -> R) -> R {
        self.packages.clear();
        let result = fill(&mut self.packages);
        self.selected = 0;
        self.rebuild_order();
        result
    }

    /// Number of packages currently visible (after the category filter).
    pub fn visible_len(&self) -> usize {
        self.order_len
    }

    pub fn sort_mode(&self) -> SortMode {
        self.sort_mode
    }

    /// The active category filter, or `None` for "all".
    pub fn category_filter(&self) -> Option<&str> {
        if self.category_filter_len == 0 {
            return None;
        }
        core::str::from_utf8(&self.category_filter[..self.category_filter_len]).ok()
    }

    /// Package index (into `packages`) at the given view position.
    fn package_index_at(&self, view: usize) -> Option<usize> {
        if view >= self.order_len {
            return None;
        }
        Some(self.order[view])
    }

    fn selected_package_index(&self) -> Option<usize> {
        self.package_index_at(self.selected)
    }

    /// Whether the currently selected package is a favorite.
    pub fn selected_is_favorite(&self) -> bool {
        self.selected_package()
            .map(|p| p.is_favorite())
            .unwrap_or(false)
    }

    /// Cycle the launcher sort order, keeping the same package selected.
    pub fn cycle_sort(&mut self) {
        let keep = self.selected_package_index();
        self.sort_mode = self.sort_mode.next();
        self.rebuild_order();
        self.reselect_package(keep);
    }

    /// Cycle the category filter (all -> each distinct category -> all),
    /// keeping the same package selected when it remains visible.
    pub fn cycle_category(&mut self) {
        let keep = self.selected_package_index();
        let mut buf = [0u8; MAX_CATEGORY_LEN];
        let mut len = 0;
        if self.next_category_into(&mut buf, &mut len) {
            self.category_filter = buf;
            self.category_filter_len = len;
        } else {
            self.category_filter_len = 0;
        }
        self.rebuild_order();
        self.reselect_package(keep);
    }

    /// Toggle the favorite flag of the selected package.
    pub fn toggle_selected_favorite(&mut self) {
        let Some(pkg_index) = self.selected_package_index() else {
            return;
        };
        if let Some(pkg) = self.packages.get_mut(pkg_index) {
            let next = !pkg.is_favorite();
            pkg.set_favorite(next);
        }
        self.rebuild_order();
        self.reselect_package(Some(pkg_index));
    }

    /// Set the favorite flag of a package by app id (used when restoring
    /// persisted state). Returns whether the package was found.
    pub fn set_favorite_by_app_id(&mut self, app_id: &str, favorite: bool) -> bool {
        for i in 0..self.packages.len() {
            let matches = self
                .packages
                .get(i)
                .map(|p| p.app_id() == app_id)
                .unwrap_or(false);
            if matches {
                if let Some(pkg) = self.packages.get_mut(i) {
                    pkg.set_favorite(favorite);
                }
                self.rebuild_order();
                return true;
            }
        }
        false
    }

    /// Set the sort mode directly (used when restoring persisted state).
    pub fn set_sort_mode(&mut self, sort_mode: SortMode) {
        let keep = self.selected_package_index();
        self.sort_mode = sort_mode;
        self.rebuild_order();
        self.reselect_package(keep);
    }

    /// Set the category filter directly (used when restoring persisted state).
    /// An unknown or empty category clears the filter.
    pub fn set_category_filter(&mut self, category: Option<&str>) {
        let keep = self.selected_package_index();
        match category {
            Some(c) if !c.is_empty() && c.len() <= MAX_CATEGORY_LEN => {
                self.category_filter[..c.len()].copy_from_slice(c.as_bytes());
                self.category_filter_len = c.len();
            }
            _ => self.category_filter_len = 0,
        }
        self.rebuild_order();
        self.reselect_package(keep);
    }

    /// Recompute the view order from the category filter and sort mode.
    fn rebuild_order(&mut self) {
        let mut tmp = [0usize; MAX_PACKAGES];
        let mut n = 0;
        let filter = self.category_filter();
        for i in 0..self.packages.len() {
            let keep = match filter {
                None => true,
                Some(f) => self.packages.get(i).and_then(PackageInfo::category) == Some(f),
            };
            if keep {
                tmp[n] = i;
                n += 1;
            }
        }
        self.sort_indices(&mut tmp[..n]);
        self.order[..n].copy_from_slice(&tmp[..n]);
        self.order_len = n;
        if self.selected >= n {
            self.selected = n.saturating_sub(1);
        }
    }

    fn sort_indices(&self, idx: &mut [usize]) {
        // Insertion sort (stable, small N, no_std-friendly).
        for i in 1..idx.len() {
            let mut j = i;
            while j > 0 && self.cmp_packages(idx[j - 1], idx[j]) == Ordering::Greater {
                idx.swap(j - 1, j);
                j -= 1;
            }
        }
    }

    fn cmp_packages(&self, a: usize, b: usize) -> Ordering {
        let (Some(pa), Some(pb)) = (self.packages.get(a), self.packages.get(b)) else {
            return Ordering::Equal;
        };
        match self.sort_mode {
            SortMode::Default => a.cmp(&b),
            SortMode::Name => pa.name().cmp(pb.name()),
            SortMode::Favorite => pb
                .is_favorite()
                .cmp(&pa.is_favorite())
                .then_with(|| pa.name().cmp(pb.name())),
        }
    }

    /// Move the selection to the given package index if it is visible, else
    /// clamp into range.
    fn reselect_package(&mut self, pkg_index: Option<usize>) {
        if let Some(target) = pkg_index {
            for view in 0..self.order_len {
                if self.order[view] == target {
                    self.selected = view;
                    return;
                }
            }
        }
        if self.selected >= self.order_len {
            self.selected = self.order_len.saturating_sub(1);
        }
    }

    /// Compute the next category filter after the current one, writing the
    /// chosen category bytes into `buf`/`len`. Returns `false` for "all".
    fn next_category_into(&self, buf: &mut [u8; MAX_CATEGORY_LEN], len: &mut usize) -> bool {
        let mut cats: [Option<&str>; MAX_PACKAGES] = [None; MAX_PACKAGES];
        let mut count = 0;
        for i in 0..self.packages.len() {
            if let Some(c) = self.packages.get(i).and_then(PackageInfo::category) {
                if !cats[..count].contains(&Some(c)) {
                    cats[count] = Some(c);
                    count += 1;
                }
            }
        }
        let pos = match self.category_filter() {
            None => 0,
            Some(f) => {
                1 + cats[..count]
                    .iter()
                    .position(|x| *x == Some(f))
                    .unwrap_or(0)
            }
        };
        let next = (pos + 1) % (count + 1);
        if next == 0 {
            *len = 0;
            return false;
        }
        let chosen = cats[next - 1].unwrap_or("");
        buf[..chosen.len()].copy_from_slice(chosen.as_bytes());
        *len = chosen.len();
        true
    }

    /// Whether the right-hand details pane is shown. When hidden, the launcher
    /// grid reclaims the full surface width (see [`shell_grid_columns`]).
    pub fn detail_pane_visible(&self) -> bool {
        self.detail_pane
    }

    pub fn set_detail_pane_visible(&mut self, visible: bool) {
        self.detail_pane = visible;
        self.pane_transition_frames = 0;
    }

    /// Toggle the details pane. The grid relayouts (its column count changes),
    /// so the selection is re-clamped and callers should repaint the full
    /// surface afterward. The page containing the selection is recomputed lazily
    /// by [`current_page`](Self::current_page).
    pub fn toggle_detail_pane(&mut self) {
        self.detail_pane = !self.detail_pane;
        self.pane_transition_frames = SHELL_PANE_TRANSITION_FRAMES;
        if self.selected >= self.order_len {
            self.selected = self.order_len.saturating_sub(1);
        }
    }

    /// Whether the system/memory status overlay is showing instead of the
    /// launcher (KOTO-0182).
    pub fn system_view_visible(&self) -> bool {
        self.system_view
    }

    pub fn set_system_view_visible(&mut self, visible: bool) {
        self.system_view = visible;
    }

    /// Toggle the system status overlay. It replaces the launcher grid, so the
    /// caller should repaint the full surface afterward.
    pub fn toggle_system_view(&mut self) {
        self.system_view = !self.system_view;
    }

    pub fn memory_status(&self) -> MemoryStatus {
        self.memory
    }

    /// Inject the latest memory snapshot for the system view. Cheap enough to
    /// call every shell frame; the overlay reads it live when open.
    pub fn set_memory_status(&mut self, memory: MemoryStatus) {
        self.memory = memory;
    }

    /// Icons shown per page for the current layout mode.
    pub fn items_per_page(&self) -> usize {
        shell_visible_items(self.detail_pane)
    }

    /// Total number of launcher pages (always at least one).
    pub fn page_count(&self) -> usize {
        let per = self.items_per_page().max(1);
        if self.order_len == 0 {
            1
        } else {
            self.order_len.div_ceil(per)
        }
    }

    /// Zero-based index of the page that contains the current selection.
    pub fn current_page(&self) -> usize {
        self.selected / self.items_per_page().max(1)
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selection_feedback_frames(&self) -> u8 {
        self.selection_feedback_frames
    }

    pub fn page_feedback_frames(&self) -> u8 {
        self.page_feedback_frames
    }

    pub fn pane_transition_frames(&self) -> u8 {
        self.pane_transition_frames
    }

    pub fn take_pending_sound(&mut self) -> Option<ShellSound> {
        self.pending_sound.take()
    }

    /// Advance the bounded shell-only visual feedback by one display frame.
    pub fn advance_feedback(&mut self) {
        self.selection_feedback_frames = self.selection_feedback_frames.saturating_sub(1);
        self.page_feedback_frames = self.page_feedback_frames.saturating_sub(1);
        self.pane_transition_frames = self.pane_transition_frames.saturating_sub(1);
    }

    pub fn selected_package(&self) -> Option<&PackageInfo> {
        self.packages.get(self.package_index_at(self.selected)?)
    }

    pub fn packages(&self) -> &PackageList {
        &self.packages
    }

    pub fn power_state(&self) -> PowerState {
        self.power_state
    }

    pub fn set_power_state(&mut self, power_state: PowerState) {
        self.power_state = power_state;
    }

    pub fn status_text(&self) -> ShellStatusText {
        ShellStatusText::from_power_state(self.power_state)
    }

    pub fn clock(&self) -> Option<ShellClock> {
        self.clock
    }

    pub fn set_clock(&mut self, clock: ShellClock) {
        self.clock = Some(clock);
    }

    pub fn clear_clock(&mut self) {
        self.clock = None;
    }

    /// `YYYY/MM/DD HH:MM`, or a `----/--/-- --:--` placeholder when unset.
    pub fn clock_text(&self) -> ShellStatusText {
        ShellStatusText::from_clock(self.clock)
    }

    pub fn storage_status(&self) -> StorageStatus {
        self.storage
    }

    pub fn set_storage_status(&mut self, storage: StorageStatus) {
        self.storage = storage;
    }

    pub fn save_status(&self) -> SaveStatus {
        self.save_status
    }

    pub fn set_save_status(&mut self, save_status: SaveStatus) {
        self.save_status = save_status;
    }

    /// Optional battery percentage for the header gauge, if the power source
    /// reports one.
    pub fn battery_percent(&self) -> Option<u8> {
        match self.power_state {
            PowerState::Charging { percent, .. } => percent,
            PowerState::Percent { percent, .. } => Some(percent),
            PowerState::Unsupported | PowerState::Unknown | PowerState::Voltage { .. } => None,
        }
    }

    pub fn battery_is_low(&self) -> bool {
        match self.power_state {
            PowerState::Percent { percent, .. } => percent <= SHELL_LOW_BATTERY_PERCENT,
            PowerState::Voltage { millivolts } => millivolts <= SHELL_LOW_BATTERY_MILLIVOLTS,
            _ => false,
        }
    }

    pub fn battery_is_charging(&self) -> bool {
        matches!(self.power_state, PowerState::Charging { .. })
    }

    /// The command bar entries for the current state. Each enabled action maps
    /// to a real input route.
    pub fn command_bar(&self) -> [ShellCommand; SHELL_COMMAND_COUNT] {
        let has_selection = self.selected_package().is_some();
        [
            ShellCommand {
                key: "Enter",
                label: "開く",
                state: None,
                enabled: has_selection,
            },
            ShellCommand {
                key: "F2",
                label: "お気に入り",
                state: Some(if self.selected_is_favorite() {
                    "★"
                } else {
                    "☆"
                }),
                enabled: has_selection,
            },
            ShellCommand {
                key: "F3",
                label: "並替",
                state: Some(self.sort_mode.label()),
                enabled: true,
            },
            ShellCommand {
                key: "F4",
                label: "カテゴリ",
                state: None,
                enabled: true,
            },
            ShellCommand {
                key: "BS",
                label: "詳細",
                state: Some(if self.detail_pane { "ON" } else { "OFF" }),
                enabled: true,
            },
            ShellCommand {
                key: "F5",
                label: "システム",
                state: None,
                enabled: true,
            },
        ]
    }

    /// Header save badge text, or `None` when the state is unknown.
    fn save_indicator_text(&self) -> Option<&'static str> {
        match self.save_status {
            SaveStatus::Saved => Some("保存済"),
            SaveStatus::Unsaved => Some("未保存"),
            SaveStatus::Unknown => None,
        }
    }

    /// Header storage badge text.
    fn storage_indicator_text(&self) -> &'static str {
        match self.storage {
            StorageStatus::Present => "SD",
            StorageStatus::Absent => "SD×",
            StorageStatus::Unknown => "SD?",
        }
    }

    pub fn update(&mut self, input: &InputState) -> ShellAction {
        if self.system_view {
            // The overlay captures input: confirm or cancel dismisses it back to
            // the launcher; navigation is ignored while it is open. Opening is a
            // host-driven route (F5 -> toggle_system_view), like sort/category.
            if input.pressed.confirm || input.pressed.cancel {
                self.system_view = false;
                self.pending_sound = Some(ShellSound::Cancel);
            }
            return ShellAction::None;
        }
        if self.order_len == 0 {
            self.selected = 0;
            if input.pressed.cancel {
                self.pending_sound = Some(ShellSound::Cancel);
            }
            return ShellAction::None;
        }

        let previous = self.selected;
        let previous_page = self.current_page();
        let columns = shell_grid_columns(self.detail_pane);
        if input.pressed.down {
            self.selected = (self.selected + columns).min(self.order_len - 1);
        }
        if input.pressed.up {
            self.selected = self.selected.saturating_sub(columns);
        }
        if input.pressed.right {
            self.selected = (self.selected + 1).min(self.order_len - 1);
        }
        if input.pressed.left {
            self.selected = self.selected.saturating_sub(1);
        }
        if self.selected != previous {
            self.selection_feedback_frames = SHELL_SELECTION_FEEDBACK_FRAMES;
            self.selection_animation_from = previous;
            if self.current_page() != previous_page {
                self.page_feedback_frames = SHELL_PAGE_FEEDBACK_FRAMES;
                self.page_animation_from = previous_page;
                // A cursor cannot meaningfully travel between different pages.
                self.selection_animation_from = self.selected;
            }
            self.pending_sound = Some(ShellSound::Navigation);
        }
        if input.pressed.cancel {
            // The details pane is toggleable in place of a separate screen; the
            // grid relayouts, so the caller should repaint the full surface.
            self.toggle_detail_pane();
            self.page_feedback_frames = SHELL_PAGE_FEEDBACK_FRAMES;
            self.pending_sound = Some(ShellSound::Cancel);
            return ShellAction::None;
        }
        if input.pressed.confirm {
            if let Some(package) = self.selected_package().copied() {
                self.pending_sound = Some(ShellSound::Confirm);
                return ShellAction::Launch(package);
            }
        }

        ShellAction::None
    }

    pub fn render_full<const N: usize>(
        &self,
        commands: &mut RenderCommandList<N>,
    ) -> Result<(), RenderError> {
        commands.push(RenderCommand::full(SHELL_SURFACE)?)
    }

    /// Render the package list as one render command per visible region: the
    /// header band followed by each on-screen package row. This drives the
    /// dirty-rectangle path (NFR-DRAW-1) instead of repainting the whole
    /// surface.
    pub fn render_list<const N: usize>(
        &self,
        commands: &mut RenderCommandList<N>,
    ) -> Result<(), RenderError> {
        let pane = self.detail_pane;
        commands.push(RenderCommand::rect(SHELL_SURFACE, shell_header_rect())?)?;

        // One command per occupied slot on the current page.
        let per = self.items_per_page();
        let start = self.current_page() * per;
        let on_page = self.order_len.saturating_sub(start).min(per);
        for slot in 0..on_page {
            commands.push(RenderCommand::rect(
                SHELL_SURFACE,
                shell_tile_rect(slot, pane),
            )?)?;
        }
        commands.push(RenderCommand::rect(
            SHELL_SURFACE,
            shell_page_indicator_rect(pane),
        )?)?;
        if pane {
            commands.push(RenderCommand::rect(SHELL_SURFACE, shell_pane_rect())?)?;
        }
        commands.push(RenderCommand::rect(
            SHELL_SURFACE,
            shell_status_strip_rect(),
        )?)?;
        commands.push(RenderCommand::rect(
            SHELL_SURFACE,
            shell_command_bar_rect(),
        )?)?;
        Ok(())
    }

    pub fn render_selection_change<const N: usize>(
        &self,
        previous_selected: usize,
        commands: &mut RenderCommandList<N>,
    ) -> Result<(), RenderError> {
        if previous_selected == self.selected {
            return Ok(());
        }

        let pane = self.detail_pane;
        let per = self.items_per_page().max(1);
        let previous_page = previous_selected / per;
        let current_page = self.selected / per;

        if previous_page == current_page {
            // Same page: only the previous and current tiles change.
            commands.push(RenderCommand::rect(
                SHELL_SURFACE,
                shell_tile_rect(previous_selected - previous_page * per, pane),
            )?)?;
            commands.push(RenderCommand::rect(
                SHELL_SURFACE,
                shell_tile_rect(self.selected - current_page * per, pane),
            )?)?;
        } else {
            // Page flip: repaint the whole grid area (tiles and page indicator).
            commands.push(RenderCommand::rect(
                SHELL_SURFACE,
                shell_grid_area_rect(pane),
            )?)?;
        }
        // The details pane and status strip both reflect the selection.
        if pane {
            commands.push(RenderCommand::rect(SHELL_SURFACE, shell_pane_rect())?)?;
        }
        commands.push(RenderCommand::rect(
            SHELL_SURFACE,
            shell_status_strip_rect(),
        )?)?;
        Ok(())
    }

    /// Rasterize the whole shell into `canvas` using [`ShellPalette::DEFAULT`].
    ///
    /// This is the host/simulator path that turns the abstract layout (the same
    /// rectangles used by [`render_list`](Self::render_list)) into real pixels so
    /// the UI can be inspected visually. On-device callers can reuse the clipped
    /// [`Canvas`] primitives to paint individual dirty tiles instead.
    pub fn paint(&self, canvas: &mut Canvas<'_>, font: &BitmapFont<'_>) {
        self.paint_with(canvas, font, &ShellPalette::DEFAULT);
    }

    pub fn paint_with(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
    ) {
        self.paint_region(canvas, font, palette, shell_surface_rect());
    }

    /// Rasterize only the portion of the shell that intersects `clip`, using
    /// [`ShellPalette::DEFAULT`].
    ///
    /// Pixels are always bounded by the [`Canvas`]; the additional `clip` lets
    /// device callers skip the raster cost of header, chrome, and off-rectangle
    /// tiles instead of rerunning the full shell painter for every transfer band
    /// (KOTO-0120 / NFR-DRAW-2). The painted pixels inside `clip` are identical
    /// to those produced by a full [`paint`](Self::paint).
    pub fn paint_rect(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        clip: crate::hal::Rect,
    ) {
        self.paint_region(canvas, font, &ShellPalette::DEFAULT, clip);
    }

    fn paint_region(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
        clip: crate::hal::Rect,
    ) {
        // Non-selected tiles and inter-component gaps rely on this background
        // fill, so it must cover the whole dirty region before any component is
        // skipped below.
        canvas.fill_rect(clip, palette.background);

        // System/memory overlay replaces the launcher body; the header still
        // paints for context (clock/battery/storage), the footer hint replaces
        // the command bar (KOTO-0182).
        if self.system_view {
            if rects_intersect(clip, shell_header_rect()) {
                self.paint_header(canvas, font, palette);
            }
            if rects_intersect(clip, shell_system_body_rect()) {
                self.paint_system_view(canvas, font, palette);
            }
            return;
        }

        let pane = self.detail_pane;
        if rects_intersect(clip, shell_header_rect()) {
            self.paint_header(canvas, font, palette);
        }
        self.paint_grid(canvas, font, palette, pane, clip);
        if (pane || self.pane_transition_frames > 0)
            && rects_intersect(clip, self.animated_pane_rect())
        {
            self.paint_pane(canvas, font, palette, self.animated_pane_rect());
        }
        if rects_intersect(clip, shell_status_strip_rect()) {
            self.paint_status_strip(canvas, font, palette);
        }
        if rects_intersect(clip, shell_command_bar_rect()) {
            self.paint_command_bar(canvas, font, palette);
        }
    }

    /// Top status bar: home label on the left, a centered clock, and system
    /// status indicators (battery, storage, save) on the right.
    fn paint_header(&self, canvas: &mut Canvas<'_>, font: &BitmapFont<'_>, palette: &ShellPalette) {
        canvas.fill_rect(shell_header_rect(), palette.header_bg);
        let ty = centered_text_y(SHELL_HEADER_HEIGHT, font);
        let mid_y = SHELL_HEADER_HEIGHT / 2;

        // Home icon and title (left).
        let icon = SHELL_HEADER_HEIGHT - 10;
        let icon_y = (SHELL_HEADER_HEIGHT - icon) / 2;
        draw_home_icon(canvas, SHELL_TEXT_PADDING, icon_y, icon, palette.header_fg);
        let title_x = SHELL_TEXT_PADDING + icon + 4;
        canvas.draw_text(title_x, ty, font, SHELL_TITLE, palette.header_fg);

        // Right cluster, laid out from the right edge leftward.
        let mut right = i32::from(SHELL_SURFACE.width) - SHELL_TEXT_PADDING;

        if let Some(text) = self.save_indicator_text() {
            let w = text_width(font, text);
            right -= w;
            let color = match self.save_status {
                SaveStatus::Saved => palette.status_ok,
                SaveStatus::Unsaved => palette.status_warn,
                SaveStatus::Unknown => palette.status_dim,
            };
            canvas.draw_text(right, ty, font, text, color);
            right -= 6;
        }

        let storage_text = self.storage_indicator_text();
        let w = text_width(font, storage_text);
        right -= w;
        let storage_color = match self.storage {
            StorageStatus::Present => palette.header_fg,
            StorageStatus::Absent => palette.status_warn,
            StorageStatus::Unknown => palette.status_dim,
        };
        canvas.draw_text(right, ty, font, storage_text, storage_color);
        right -= 8;

        right = self.paint_battery(canvas, font, palette, right, mid_y, ty);

        // Centered clock, kept clear of the side clusters.
        let clock = self.clock_text();
        if let Some(text) = clock.as_str() {
            let cw = text_width(font, text);
            let cx = (i32::from(SHELL_SURFACE.width) - cw) / 2;
            let clock_color = if self.clock.is_some() {
                palette.header_fg
            } else {
                palette.status_dim
            };
            if cx + cw < right {
                canvas.draw_text(cx, ty, font, text, clock_color);
            }
        }
    }

    /// Paint the battery gauge and its percentage just left of `right`. Returns
    /// the new left edge of the cluster.
    fn paint_battery(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
        right: i32,
        mid_y: i32,
        ty: i32,
    ) -> i32 {
        let mut right = right;
        match self.battery_percent() {
            Some(percent) => {
                let mut buf = ShellDetailText::empty();
                buf.push_u32(u32::from(percent));
                buf.push_str("%");
                let text = buf.as_str();
                let w = text_width(font, text);
                right -= w;
                canvas.draw_text(right, ty, font, text, palette.header_fg);
                right -= 4;
            }
            None => {
                let text = "--%";
                let w = text_width(font, text);
                right -= w;
                canvas.draw_text(right, ty, font, text, palette.status_dim);
                right -= 4;
            }
        }

        let body_w = 14;
        let body_h = 8;
        right -= body_w + 2; // 2px for the terminal nub
        let fill = if self.battery_is_low() {
            palette.status_warn
        } else {
            palette.status_ok
        };
        draw_battery_icon(
            canvas,
            right,
            mid_y - body_h / 2,
            body_w,
            body_h,
            self.battery_percent(),
            palette.header_fg,
            fill,
            palette.status_dim,
        );
        right - 8
    }

    /// Launcher icon grid (current page) for the current layout mode.
    fn paint_grid(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
        pane: bool,
        clip: crate::hal::Rect,
    ) {
        let current_page = self.current_page();
        if let Some((from_page, from_dx, current_dx)) = self.page_slide_offsets(pane) {
            self.paint_grid_page(canvas, font, palette, pane, from_page, from_dx, false, clip);
            self.paint_grid_page(
                canvas,
                font,
                palette,
                pane,
                current_page,
                current_dx,
                true,
                clip,
            );
        } else {
            self.paint_grid_page(canvas, font, palette, pane, current_page, 0, true, clip);
        }
        if rects_intersect(clip, shell_page_indicator_rect(pane)) {
            self.paint_page_indicator(canvas, font, palette, pane);
        }
    }

    fn page_slide_offsets(&self, pane: bool) -> Option<(usize, i32, i32)> {
        let current_page = self.current_page();
        if self.page_feedback_frames == 0 || self.page_animation_from == current_page {
            return None;
        }
        let width = shell_grid_area_width(pane);
        let total = i32::from(SHELL_PAGE_FEEDBACK_FRAMES);
        let remaining = i32::from(self.page_feedback_frames);
        let denominator = total * total;
        let progress = denominator - remaining * remaining;
        let distance = lerp_i32(0, width, progress, denominator);
        let direction = if current_page > self.page_animation_from {
            1
        } else {
            -1
        };
        Some((
            self.page_animation_from,
            -direction * distance,
            direction * (width - distance),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_grid_page(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
        pane: bool,
        page: usize,
        x_offset: i32,
        show_cursor: bool,
        clip: crate::hal::Rect,
    ) {
        let per = shell_visible_items(pane);
        let start = page * per;
        let on_page = self.order_len.saturating_sub(start).min(per);
        for slot in 0..on_page {
            let view = start + slot;
            let rect = translate_rect(shell_tile_rect(slot, pane), x_offset, 0);
            if !rects_intersect(clip, rect) {
                continue;
            }
            let selected = view == self.selected;
            if selected {
                let pulse = self.selection_feedback_frames > 0;
                canvas.fill_rect(
                    rect,
                    if pulse {
                        Rgb565::from_rgb8(238, 247, 255)
                    } else {
                        palette.selected_bg
                    },
                );
            }
            if let Some(package) = self
                .package_index_at(view)
                .and_then(|i| self.packages.get(i))
            {
                let fg = if selected {
                    palette.selected_fg
                } else {
                    palette.row_fg
                };
                draw_package_icon(canvas, icon_rect(rect), package, palette, selected);
                // Favorite marker in the tile's top-left corner.
                if package.is_favorite() {
                    canvas.draw_text(rect.x + 3, rect.y + 4, font, "★", palette.pane_accent);
                }
                // Clip the label to the tile width, then center it.
                let max_w = rect.w - 2 * SHELL_TEXT_PADDING;
                let label = clip_text_to_width(font, package.name(), max_w);
                let label_w = text_width(font, label);
                let lx = (rect.x + (rect.w - label_w) / 2).max(rect.x + 2);
                canvas.draw_text(lx, rect.y + SHELL_ICON_SIZE + 14, font, label, fg);
            }
        }
        if show_cursor && self.order_len > 0 {
            let cursor = translate_rect(self.selection_cursor_rect(pane), x_offset, 0);
            if !rects_intersect(clip, cursor) {
                return;
            }
            draw_border(canvas, cursor, palette.selected_border, 2);
            if self.selection_feedback_frames > 0 {
                draw_border(
                    canvas,
                    crate::hal::Rect {
                        x: cursor.x + 3,
                        y: cursor.y + 3,
                        w: cursor.w - 6,
                        h: cursor.h - 6,
                    },
                    palette.pane_accent,
                    1,
                );
            }
        }
    }

    fn animated_pane_rect(&self) -> crate::hal::Rect {
        let target = shell_pane_rect();
        if self.pane_transition_frames == 0 {
            return target;
        }
        let total = i32::from(SHELL_PANE_TRANSITION_FRAMES);
        let remaining = i32::from(self.pane_transition_frames);
        let denominator = total * total;
        let progress = denominator - remaining * remaining;
        let hidden_x = i32::from(SHELL_SURFACE.width);
        let x = if self.detail_pane {
            lerp_i32(hidden_x, target.x, progress, denominator)
        } else {
            lerp_i32(target.x, hidden_x, progress, denominator)
        };
        crate::hal::Rect { x, ..target }
    }

    /// Animated selection outline. It uses integer ease-out interpolation and
    /// stores only the source slot plus a small frame counter.
    fn selection_cursor_rect(&self, pane: bool) -> crate::hal::Rect {
        let per = shell_visible_items(pane).max(1);
        let current_page = self.selected / per;
        let from_page = self.selection_animation_from / per;
        let to = shell_tile_rect(self.selected - current_page * per, pane);
        if self.selection_feedback_frames == 0 || from_page != current_page {
            return to;
        }

        let from = shell_tile_rect(self.selection_animation_from - from_page * per, pane);
        let total = i32::from(SHELL_SELECTION_FEEDBACK_FRAMES);
        let remaining = i32::from(self.selection_feedback_frames);
        let denominator = total * total;
        let progress = denominator - remaining * remaining;
        crate::hal::Rect {
            x: lerp_i32(from.x, to.x, progress, denominator),
            y: lerp_i32(from.y, to.y, progress, denominator),
            w: lerp_i32(from.w, to.w, progress, denominator),
            h: lerp_i32(from.h, to.h, progress, denominator),
        }
    }

    /// Page indicator (left/right triangles flanking page numbers) centered below
    /// the grid, with the current page accented.
    fn paint_page_indicator(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
        pane: bool,
    ) {
        let total = self.page_count();
        let current = self.current_page();
        let rect = shell_page_indicator_rect(pane);
        if self.page_feedback_frames > 0 {
            canvas.fill_rect(
                crate::hal::Rect {
                    x: rect.x,
                    y: rect.y,
                    w: rect.w,
                    h: 2,
                },
                palette.pane_accent,
            );
        }
        let cell = i32::from(font.cell_height());
        let ty = rect.y + ((rect.h - cell) / 2).max(0);
        let cy = rect.y + rect.h / 2;
        let gap = i32::from(font.half_width());
        let arrow_w = SHELL_PAGE_ARROW_WIDTH;
        let arrow_h = (cell - 2).max(4);

        let mut numbers_w = 0;
        for page in 1..=total {
            numbers_w += u32_text_width(font, page as u32) + gap;
        }
        let total_w = arrow_w + gap + numbers_w + arrow_w;
        let mut x = (rect.x + (rect.w - total_w) / 2).max(rect.x + 2);

        draw_triangle(
            canvas,
            x,
            cy,
            arrow_w,
            arrow_h,
            false,
            palette.status_strip_fg,
        );
        x += arrow_w + gap;
        for page in 1..=total {
            let color = if page - 1 == current {
                palette.pane_accent
            } else {
                palette.status_strip_fg
            };
            let mut buf = ShellDetailText::empty();
            buf.push_u32(page as u32);
            x = canvas.draw_text(x, ty, font, buf.as_str(), color) + gap;
        }
        draw_triangle(
            canvas,
            x,
            cy,
            arrow_w,
            arrow_h,
            true,
            palette.status_strip_fg,
        );
    }

    /// Right-hand details pane: selected-app name, favorite star, wrapped
    /// description, and metadata slots (last opened, size, category, favorite).
    /// Slots without real data yet show deterministic placeholders.
    fn paint_pane(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
        rect: crate::hal::Rect,
    ) {
        canvas.fill_rect(rect, palette.pane_bg);
        canvas.fill_rect(
            crate::hal::Rect {
                x: rect.x,
                y: rect.y,
                w: 1,
                h: rect.h,
            },
            palette.separator,
        );

        let Some(package) = self.selected_package() else {
            return;
        };
        let x = rect.x + SHELL_TEXT_PADDING + 2;
        let content_w = rect.w - (x - rect.x) - SHELL_TEXT_PADDING;
        let cell = i32::from(font.cell_height());
        let line_h = cell + 2;
        let mut y = rect.y + 6;

        // Title: favorite star + name.
        let mut name_x = x;
        if package.is_favorite() {
            name_x = canvas.draw_text(x, y, font, "★", palette.pane_accent) + 2;
        }
        let name_w = content_w - (name_x - x);
        let name = clip_text_to_width(font, package.name(), name_w);
        canvas.draw_text(name_x, y, font, name, palette.pane_accent);
        y += cell + 3;
        canvas.fill_rect(
            crate::hal::Rect {
                x,
                y,
                w: content_w,
                h: 1,
            },
            palette.separator,
        );
        y += 5;

        // Description, wrapped to the pane width.
        let description = package.description().unwrap_or("(説明なし)");
        y = draw_wrapped_text(
            canvas,
            x,
            y,
            content_w,
            line_h,
            3,
            font,
            description,
            palette.pane_fg,
        );
        y += 4;

        // Metadata slots. Category is real (KOTO-0091); the rest are
        // deterministic placeholders until their data sources exist.
        y = paint_pane_slot(
            canvas,
            font,
            palette,
            x,
            y,
            content_w,
            "最後に開いた日時",
            "----/--/-- --:--",
        );
        y = paint_pane_slot(canvas, font, palette, x, y, content_w, "サイズ", "-- KB");
        let category = package.category().unwrap_or("--");
        y = paint_pane_slot(canvas, font, palette, x, y, content_w, "カテゴリ", category);
        let favorite = if package.is_favorite() {
            "★ はい"
        } else {
            "☆ いいえ"
        };
        let _ = paint_pane_slot(
            canvas,
            font,
            palette,
            x,
            y,
            content_w,
            "お気に入り",
            favorite,
        );
    }

    /// System/memory status overlay body (KOTO-0182). Draws labelled rows for
    /// SRAM headroom, stack peaks, app heap, and PSRAM into the region below the
    /// header, plus a footer hint on how to return to the launcher.
    fn paint_system_view(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
    ) {
        let body = shell_system_body_rect();
        canvas.fill_rect(body, palette.pane_bg);

        let m = self.memory;
        let cell = i32::from(font.cell_height());
        let line_h = cell + 5;
        let label_x = body.x + SHELL_TEXT_PADDING + 4;
        let value_x = body.x + 132;
        let mut y = body.y + 8;

        canvas.draw_text(label_x, y, font, "システム / メモリ", palette.pane_accent);
        y += line_h + 3;

        // SRAM headroom is the headline; colour it by the KOTO-0170 margin.
        let free_color = match m.sram_free_min {
            Some(free) if free < MemoryStatus::FREE_MIN_CAUTION_BYTES => palette.status_warn,
            Some(_) => palette.status_ok,
            None => palette.status_dim,
        };
        let mut sram = mem_kib_text(m.sram_free_min);
        sram.push_str(" / ");
        if m.sram_total > 0 {
            sram.push_kib(m.sram_total);
        } else {
            sram.push_str("----");
        }
        draw_mem_row(
            canvas,
            font,
            label_x,
            value_x,
            y,
            "SRAM 空き最小",
            sram.as_str(),
            palette.pane_fg,
            free_color,
        );
        y += line_h;

        draw_mem_row(
            canvas,
            font,
            label_x,
            value_x,
            y,
            "  静的常駐",
            mem_kib_text(m.sram_static_used).as_str(),
            palette.pane_fg,
            palette.pane_fg,
        );
        y += line_h;

        draw_mem_row(
            canvas,
            font,
            label_x,
            value_x,
            y,
            "  スタック最大",
            mem_kib_text(m.stack_peak_used).as_str(),
            palette.pane_fg,
            palette.pane_fg,
        );
        y += line_h;

        draw_mem_row(
            canvas,
            font,
            label_x,
            value_x,
            y,
            "Core1 音声空き",
            mem_kib_text(m.core1_stack_free_min).as_str(),
            palette.pane_fg,
            palette.pane_fg,
        );
        y += line_h;

        let mut heap = mem_kib_text(m.app_heap_last_used);
        heap.push_str(" / ");
        match m.app_heap_total {
            Some(total) => heap.push_kib(total),
            None => heap.push_str("----"),
        }
        draw_mem_row(
            canvas,
            font,
            label_x,
            value_x,
            y,
            "アプリ heap",
            heap.as_str(),
            palette.pane_fg,
            palette.pane_fg,
        );
        y += line_h;

        let mut psram = ShellDetailText::empty();
        if m.psram_present {
            psram.push_mib(m.psram_total);
            psram.push_str("  窓");
            psram.push_u32(u32::from(m.code_window_slots));
        } else {
            psram.push_str("----（なし）");
        }
        let psram_color = if m.psram_present {
            palette.status_ok
        } else {
            palette.status_dim
        };
        draw_mem_row(
            canvas,
            font,
            label_x,
            value_x,
            y,
            "PSRAM",
            psram.as_str(),
            palette.pane_fg,
            psram_color,
        );

        // Footer hint, bottom-aligned in the body band.
        let copyright_y = body.y + body.h - cell * 2 - 8;
        let copyright_w = text_width(font, KOTO_COPYRIGHT_NOTICE);
        canvas.draw_text(
            (body.x + body.w - SHELL_TEXT_PADDING - copyright_w).max(label_x),
            copyright_y,
            font,
            KOTO_COPYRIGHT_NOTICE,
            palette.status_dim,
        );
        let hint_y = body.y + body.h - cell - 4;
        canvas.draw_text(
            label_x,
            hint_y,
            font,
            "F5 / Enter で戻る",
            palette.status_dim,
        );
    }

    /// Secondary status strip: sort/category state and selection / page summary.
    fn paint_status_strip(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
    ) {
        let rect = shell_status_strip_rect();
        canvas.fill_rect(rect, palette.status_strip_bg);
        let cell = i32::from(font.cell_height());
        let ty = rect.y + ((rect.h - cell) / 2).max(0);

        // Left: sort mode and category filter.
        let mut left = ShellDetailText::empty();
        left.push_str("並び:");
        left.push_str(self.sort_mode.label());
        left.push_str("  分類:");
        left.push_str(self.category_filter().unwrap_or("全部"));
        canvas.draw_text(
            rect.x + SHELL_TEXT_PADDING,
            ty,
            font,
            left.as_str(),
            palette.status_strip_fg,
        );

        // Right: selection / page summary.
        let mut line = ShellDetailText::empty();
        if let Some(package) = self.selected_package() {
            line.push_str("選択中:");
            line.push_str(package.name());
            line.push_str(" (");
            line.push_u32((self.selected + 1) as u32);
            line.push_str("/");
            line.push_u32(self.order_len as u32);
            line.push_str(")");
        }
        let text = line.as_str();
        if !text.is_empty() {
            let w = text_width(font, text);
            let x = i32::from(SHELL_SURFACE.width) - SHELL_TEXT_PADDING - w;
            canvas.draw_text(
                x.max(rect.x + SHELL_TEXT_PADDING),
                ty,
                font,
                text,
                palette.status_strip_fg,
            );
        }
    }

    /// Bottom command bar listing the available shell actions. Action wiring is
    /// completed by KOTO-0085.
    fn paint_command_bar(
        &self,
        canvas: &mut Canvas<'_>,
        font: &BitmapFont<'_>,
        palette: &ShellPalette,
    ) {
        let rect = shell_command_bar_rect();
        canvas.fill_rect(rect, palette.command_bar_bg);
        let cell = i32::from(font.cell_height());
        let ty = rect.y + ((rect.h - cell) / 2).max(0);
        let limit = rect.x + rect.w - SHELL_TEXT_PADDING;
        let mut x = rect.x + SHELL_TEXT_PADDING;

        for cmd in self.command_bar() {
            let key_w = text_width(font, cmd.key) + 4;
            let mut label_w = text_width(font, cmd.label);
            if let Some(state) = cmd.state {
                label_w += 2 + text_width(font, state);
            }
            // Break at a command boundary rather than clipping mid-label.
            if x + key_w + 2 + label_w > limit {
                break;
            }

            let (key_fg, label_fg) = if cmd.enabled {
                (palette.command_bar_fg, palette.command_bar_fg)
            } else {
                (palette.status_dim, palette.status_dim)
            };
            if cmd.enabled {
                canvas.fill_rect(
                    crate::hal::Rect {
                        x,
                        y: rect.y + 2,
                        w: key_w,
                        h: rect.h - 4,
                    },
                    palette.command_key_bg,
                );
            }
            canvas.draw_text(x + 2, ty, font, cmd.key, key_fg);
            x += key_w + 2;
            x = canvas.draw_text(x, ty, font, cmd.label, label_fg);
            if let Some(state) = cmd.state {
                x = canvas.draw_text(x + 2, ty, font, state, palette.status_ok);
            }
            x += 8;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellStatusText {
    bytes: [u8; SHELL_STATUS_TEXT_CAPACITY],
    len: usize,
}

impl ShellStatusText {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; SHELL_STATUS_TEXT_CAPACITY],
            len: 0,
        }
    }

    pub fn from_power_state(power_state: PowerState) -> Self {
        let mut text = Self::empty();
        match power_state {
            PowerState::Unsupported => {}
            PowerState::Unknown => text.push_str("BAT ?"),
            PowerState::Charging { percent, .. } => {
                text.push_str("CHG");
                if let Some(percent) = percent {
                    text.push_str(" ");
                    text.push_u8(percent);
                    text.push_str("%");
                }
            }
            PowerState::Percent { percent, .. } => {
                if percent <= SHELL_LOW_BATTERY_PERCENT {
                    text.push_str("LOW ");
                } else {
                    text.push_str("BAT ");
                }
                text.push_u8(percent);
                text.push_str("%");
            }
            PowerState::Voltage { millivolts } => {
                if millivolts <= SHELL_LOW_BATTERY_MILLIVOLTS {
                    text.push_str("LOW ");
                } else {
                    text.push_str("BAT ");
                }
                text.push_u16(millivolts);
                text.push_str("mV");
            }
        }
        text
    }

    /// `YYYY/MM/DD HH:MM`, or a `----/--/-- --:--` placeholder when `clock` is
    /// `None`. The fixed 16-character form fits [`SHELL_STATUS_TEXT_CAPACITY`].
    pub fn from_clock(clock: Option<ShellClock>) -> Self {
        let mut text = Self::empty();
        match clock {
            None => text.push_str("----/--/-- --:--"),
            Some(clock) => {
                text.push_u16_pad(clock.year, 4);
                text.push_str("/");
                text.push_u16_pad(u16::from(clock.month), 2);
                text.push_str("/");
                text.push_u16_pad(u16::from(clock.day), 2);
                text.push_str(" ");
                text.push_u16_pad(u16::from(clock.hour), 2);
                text.push_str(":");
                text.push_u16_pad(u16::from(clock.minute), 2);
            }
        }
        text
    }

    pub fn as_str(&self) -> Option<&str> {
        if self.len == 0 {
            return None;
        }
        core::str::from_utf8(&self.bytes[..self.len]).ok()
    }

    fn push_str(&mut self, value: &str) {
        for byte in value.bytes() {
            self.push_byte(byte);
        }
    }

    /// Push `value` as decimal digits, left-padded with `0` to at least `width`.
    fn push_u16_pad(&mut self, value: u16, width: usize) {
        let mut digits = [0u8; 5];
        let mut len = 0;
        let mut remaining = value;
        loop {
            digits[len] = b'0' + (remaining % 10) as u8;
            len += 1;
            remaining /= 10;
            if remaining == 0 {
                break;
            }
        }
        for _ in len..width {
            self.push_byte(b'0');
        }
        for index in (0..len).rev() {
            self.push_byte(digits[index]);
        }
    }

    fn push_u8(&mut self, value: u8) {
        self.push_u16(u16::from(value));
    }

    fn push_u16(&mut self, mut value: u16) {
        let mut digits = [0u8; 5];
        let mut len = 0;
        loop {
            digits[len] = b'0' + (value % 10) as u8;
            len += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        for index in (0..len).rev() {
            self.push_byte(digits[index]);
        }
    }

    fn push_byte(&mut self, byte: u8) {
        if self.len < SHELL_STATUS_TEXT_CAPACITY {
            self.bytes[self.len] = byte;
            self.len += 1;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShellDetailText {
    bytes: [u8; SHELL_DETAIL_TEXT_CAPACITY],
    len: usize,
}

impl ShellDetailText {
    const fn empty() -> Self {
        Self {
            bytes: [0; SHELL_DETAIL_TEXT_CAPACITY],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }

    fn push_str(&mut self, value: &str) {
        for byte in value.bytes() {
            if self.len < SHELL_DETAIL_TEXT_CAPACITY {
                self.bytes[self.len] = byte;
                self.len += 1;
            }
        }
    }

    fn push_u32(&mut self, mut value: u32) {
        let mut digits = [0u8; 10];
        let mut len = 0;
        loop {
            digits[len] = b'0' + (value % 10) as u8;
            len += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        for index in (0..len).rev() {
            if self.len < SHELL_DETAIL_TEXT_CAPACITY {
                self.bytes[self.len] = digits[index];
                self.len += 1;
            }
        }
    }

    /// Append `bytes` as a compact `"<whole>.<tenth>KiB"` string (KOTO-0182).
    /// SRAM figures stay well under 1 MiB, so one decimal place is plenty.
    fn push_kib(&mut self, bytes: usize) {
        self.push_u32((bytes / 1024) as u32);
        self.push_str(".");
        self.push_u32(((bytes % 1024) * 10 / 1024) as u32);
        self.push_str("KiB");
    }

    /// Append `bytes` as whole `"<n>MiB"` (used for PSRAM totals).
    fn push_mib(&mut self, bytes: usize) {
        self.push_u32((bytes / (1024 * 1024)) as u32);
        self.push_str("MiB");
    }
}

/// A `KiB` value string, or `"----"` when the host has not measured it yet.
fn mem_kib_text(bytes: Option<usize>) -> ShellDetailText {
    let mut text = ShellDetailText::empty();
    match bytes {
        Some(value) => text.push_kib(value),
        None => text.push_str("----"),
    }
    text
}

/// Label + value on one system-view row.
#[allow(clippy::too_many_arguments)]
fn draw_mem_row(
    canvas: &mut Canvas<'_>,
    font: &BitmapFont<'_>,
    label_x: i32,
    value_x: i32,
    y: i32,
    label: &str,
    value: &str,
    label_color: Rgb565,
    value_color: Rgb565,
) {
    canvas.draw_text(label_x, y, font, label, label_color);
    canvas.draw_text(value_x, y, font, value, value_color);
}

/// Draw `text` wrapped to `max_w`, advancing `line_h` per line, up to
/// `max_lines`. Returns the y position below the last drawn line. Wrapping is
/// per-character, which suits Japanese text and is acceptable for short labels.
#[allow(clippy::too_many_arguments)]
fn draw_wrapped_text(
    canvas: &mut Canvas<'_>,
    x: i32,
    mut y: i32,
    max_w: i32,
    line_h: i32,
    max_lines: usize,
    font: &BitmapFont<'_>,
    text: &str,
    color: Rgb565,
) -> i32 {
    let mut start = 0;
    let mut width = 0;
    let mut drawn = 0;
    let mut last = 0;
    for (offset, ch) in text.char_indices() {
        let glyph_w = font
            .glyph(ch)
            .map(|glyph| glyph.width() as i32)
            .unwrap_or_else(|| font.half_width() as i32);
        if width + glyph_w > max_w && offset > start {
            canvas.draw_text(x, y, font, &text[start..offset], color);
            y += line_h;
            drawn += 1;
            if drawn >= max_lines {
                return y;
            }
            start = offset;
            width = 0;
        }
        width += glyph_w;
        last = offset + ch.len_utf8();
    }
    if start < last {
        canvas.draw_text(x, y, font, &text[start..last], color);
        y += line_h;
    }
    y
}

/// Draw a labeled metadata slot (dashed separator, dim label, value) in the
/// details pane. Returns the y position below the slot.
#[allow(clippy::too_many_arguments)]
fn paint_pane_slot(
    canvas: &mut Canvas<'_>,
    font: &BitmapFont<'_>,
    palette: &ShellPalette,
    x: i32,
    y: i32,
    w: i32,
    label: &str,
    value: &str,
) -> i32 {
    let cell = i32::from(font.cell_height());
    let mut y = y;
    draw_dashed_hline(canvas, x, y, w, palette.separator);
    y += 4;
    canvas.draw_text(x, y, font, label, palette.status_dim);
    y += cell;
    let value = clip_text_to_width(font, value, w);
    canvas.draw_text(x, y, font, value, palette.pane_fg);
    y + cell + 2
}

/// Draw a dashed horizontal line (2px dashes every 4px).
fn draw_dashed_hline(canvas: &mut Canvas<'_>, x: i32, y: i32, w: i32, color: Rgb565) {
    let mut dx = 0;
    while dx < w {
        canvas.fill_rect(
            crate::hal::Rect {
                x: x + dx,
                y,
                w: 2,
                h: 1,
            },
            color,
        );
        dx += 4;
    }
}

/// Vertical offset that centers a glyph cell within a band of `band_h` pixels.
fn centered_text_y(band_h: i32, font: &BitmapFont<'_>) -> i32 {
    let cell = font.cell_height() as i32;
    ((band_h - cell) / 2).max(0)
}

fn lerp_i32(from: i32, to: i32, numerator: i32, denominator: i32) -> i32 {
    from + (to - from) * numerator / denominator.max(1)
}

fn translate_rect(rect: crate::hal::Rect, dx: i32, dy: i32) -> crate::hal::Rect {
    crate::hal::Rect {
        x: rect.x + dx,
        y: rect.y + dy,
        ..rect
    }
}

fn text_width(font: &BitmapFont<'_>, text: &str) -> i32 {
    text.chars()
        .map(|ch| {
            font.glyph(ch)
                .map(|glyph| glyph.width() as i32)
                .unwrap_or_else(|| font.half_width() as i32)
        })
        .sum()
}

/// Width of `value` rendered as decimal digits.
fn u32_text_width(font: &BitmapFont<'_>, value: u32) -> i32 {
    let mut buf = ShellDetailText::empty();
    buf.push_u32(value);
    text_width(font, buf.as_str())
}

/// Longest prefix of `text` whose rendered width fits within `max_w` pixels.
fn clip_text_to_width<'a>(font: &BitmapFont<'_>, text: &'a str, max_w: i32) -> &'a str {
    let mut width = 0;
    let mut end = 0;
    for (offset, ch) in text.char_indices() {
        let glyph_w = font
            .glyph(ch)
            .map(|glyph| glyph.width() as i32)
            .unwrap_or_else(|| font.half_width() as i32);
        if width + glyph_w > max_w {
            break;
        }
        width += glyph_w;
        end = offset + ch.len_utf8();
    }
    &text[..end]
}

/// The full shell surface rectangle.
fn shell_surface_rect() -> crate::hal::Rect {
    crate::hal::Rect {
        x: 0,
        y: 0,
        w: i32::from(SHELL_SURFACE.width),
        h: i32::from(SHELL_SURFACE.height),
    }
}

/// Whether two rectangles overlap in at least one pixel.
fn rects_intersect(a: crate::hal::Rect, b: crate::hal::Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn shell_header_rect() -> crate::hal::Rect {
    crate::hal::Rect {
        x: 0,
        y: 0,
        w: i32::from(SHELL_SURFACE.width),
        h: SHELL_HEADER_HEIGHT,
    }
}

/// Width available to the launcher grid for the current layout mode.
fn shell_grid_area_width(pane_visible: bool) -> i32 {
    if pane_visible {
        i32::from(SHELL_SURFACE.width) - SHELL_PANE_WIDTH
    } else {
        i32::from(SHELL_SURFACE.width)
    }
}

fn shell_tile_rect(index: usize, pane_visible: bool) -> crate::hal::Rect {
    let columns = shell_grid_columns(pane_visible);
    let column = index % columns;
    let row = index / columns;
    let tile_width = shell_grid_area_width(pane_visible) / columns as i32;
    crate::hal::Rect {
        x: column as i32 * tile_width,
        y: SHELL_HEADER_HEIGHT + (row as i32 * SHELL_TILE_HEIGHT),
        w: tile_width,
        h: SHELL_TILE_HEIGHT,
    }
}

/// The whole launcher grid region (icon rows plus the page-indicator band).
fn shell_grid_area_rect(pane_visible: bool) -> crate::hal::Rect {
    crate::hal::Rect {
        x: 0,
        y: SHELL_HEADER_HEIGHT,
        w: shell_grid_area_width(pane_visible),
        h: SHELL_GRID_AREA_HEIGHT,
    }
}

/// Band below the icon rows that holds the page indicator.
fn shell_page_indicator_rect(pane_visible: bool) -> crate::hal::Rect {
    let rows_height = SHELL_GRID_ROWS as i32 * SHELL_TILE_HEIGHT;
    crate::hal::Rect {
        x: 0,
        y: SHELL_HEADER_HEIGHT + rows_height,
        w: shell_grid_area_width(pane_visible),
        h: SHELL_GRID_AREA_HEIGHT - rows_height,
    }
}

fn shell_pane_rect() -> crate::hal::Rect {
    crate::hal::Rect {
        x: i32::from(SHELL_SURFACE.width) - SHELL_PANE_WIDTH,
        y: SHELL_HEADER_HEIGHT,
        w: SHELL_PANE_WIDTH,
        h: SHELL_GRID_AREA_HEIGHT,
    }
}

fn shell_status_strip_rect() -> crate::hal::Rect {
    crate::hal::Rect {
        x: 0,
        y: i32::from(SHELL_SURFACE.height) - SHELL_FOOTER_HEIGHT,
        w: i32::from(SHELL_SURFACE.width),
        h: SHELL_STATUS_STRIP_HEIGHT,
    }
}

/// Body region of the system/memory overlay: everything below the header
/// (KOTO-0182). The overlay draws its own footer hint inside this band.
fn shell_system_body_rect() -> crate::hal::Rect {
    crate::hal::Rect {
        x: 0,
        y: SHELL_HEADER_HEIGHT,
        w: i32::from(SHELL_SURFACE.width),
        h: i32::from(SHELL_SURFACE.height) - SHELL_HEADER_HEIGHT,
    }
}

fn shell_command_bar_rect() -> crate::hal::Rect {
    crate::hal::Rect {
        x: 0,
        y: i32::from(SHELL_SURFACE.height) - SHELL_COMMAND_BAR_HEIGHT,
        w: i32::from(SHELL_SURFACE.width),
        h: SHELL_COMMAND_BAR_HEIGHT,
    }
}

/// Draw a filled left- or right-pointing triangle of width `w` and height `h`,
/// vertically centered on `y_center`, with its left edge at `x`.
fn draw_triangle(
    canvas: &mut Canvas<'_>,
    x: i32,
    y_center: i32,
    w: i32,
    h: i32,
    point_right: bool,
    color: Rgb565,
) {
    let half_max = h / 2;
    for dx in 0..w {
        let half = if point_right {
            half_max * (w - dx) / w
        } else {
            half_max * dx / w
        };
        canvas.fill_rect(
            crate::hal::Rect {
                x: x + dx,
                y: y_center - half,
                w: 1,
                h: 2 * half + 1,
            },
            color,
        );
    }
}

/// Draw a small house glyph (triangular roof over a square body) in `color`,
/// fitting a `size`x`size` box at (`x`, `y`).
fn draw_home_icon(canvas: &mut Canvas<'_>, x: i32, y: i32, size: i32, color: Rgb565) {
    let roof_h = size / 2;
    // Roof: a filled triangle pointing up.
    for row in 0..roof_h {
        let half = (row + 1) * (size / 2) / roof_h;
        canvas.fill_rect(
            crate::hal::Rect {
                x: x + size / 2 - half,
                y: y + row,
                w: 2 * half,
                h: 1,
            },
            color,
        );
    }
    // Body.
    let body_x = x + size / 4;
    canvas.fill_rect(
        crate::hal::Rect {
            x: body_x,
            y: y + roof_h,
            w: size - size / 2,
            h: size - roof_h,
        },
        color,
    );
}

/// Draw a battery gauge: an outlined body filled proportionally to `percent`
/// (or hollow when `None`) plus a terminal nub on the right.
#[allow(clippy::too_many_arguments)]
fn draw_battery_icon(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    percent: Option<u8>,
    outline: Rgb565,
    fill: Rgb565,
    empty: Rgb565,
) {
    // Body outline.
    draw_border(canvas, crate::hal::Rect { x, y, w, h }, outline, 1);
    // Terminal nub.
    canvas.fill_rect(
        crate::hal::Rect {
            x: x + w,
            y: y + h / 4,
            w: 2,
            h: h - h / 2,
        },
        outline,
    );
    // Charge level.
    let inner_w = w - 2;
    let inner_h = h - 2;
    match percent {
        Some(percent) => {
            let filled = inner_w * i32::from(percent.min(100)) / 100;
            if filled > 0 {
                canvas.fill_rect(
                    crate::hal::Rect {
                        x: x + 1,
                        y: y + 1,
                        w: filled,
                        h: inner_h,
                    },
                    fill,
                );
            }
        }
        None => {
            // A dim hatch mark for "unknown".
            canvas.fill_rect(
                crate::hal::Rect {
                    x: x + 1,
                    y: y + h / 2,
                    w: inner_w,
                    h: 1,
                },
                empty,
            );
        }
    }
}

/// Draw a `thickness`-pixel border just inside `rect`.
fn draw_border(canvas: &mut Canvas<'_>, rect: crate::hal::Rect, color: Rgb565, thickness: i32) {
    let t = thickness.max(1);
    canvas.fill_rect(
        crate::hal::Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: t,
        },
        color,
    );
    canvas.fill_rect(
        crate::hal::Rect {
            x: rect.x,
            y: rect.y + rect.h - t,
            w: rect.w,
            h: t,
        },
        color,
    );
    canvas.fill_rect(
        crate::hal::Rect {
            x: rect.x,
            y: rect.y,
            w: t,
            h: rect.h,
        },
        color,
    );
    canvas.fill_rect(
        crate::hal::Rect {
            x: rect.x + rect.w - t,
            y: rect.y,
            w: t,
            h: rect.h,
        },
        color,
    );
}

fn icon_rect(tile: crate::hal::Rect) -> crate::hal::Rect {
    crate::hal::Rect {
        x: tile.x + (tile.w - SHELL_ICON_SIZE) / 2,
        y: tile.y + 8,
        w: SHELL_ICON_SIZE,
        h: SHELL_ICON_SIZE,
    }
}

/// A built-in colored launcher icon. Maps to a coherent placeholder icon set
/// keyed by the package's category or app id (KOTO-0087).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconKind {
    Notepad,
    Calendar,
    Folder,
    Calculator,
    Gear,
    Music,
    Game,
    Terminal,
}

const ICON_KINDS: [IconKind; 8] = [
    IconKind::Notepad,
    IconKind::Calendar,
    IconKind::Folder,
    IconKind::Calculator,
    IconKind::Gear,
    IconKind::Music,
    IconKind::Game,
    IconKind::Terminal,
];

/// Choose a launcher icon for `package` by its declared category, then by a
/// stable hash so uncategorized apps still get visually distinct icons.
pub fn icon_kind_for(package: &PackageInfo) -> IconKind {
    let id = package.app_id();
    let category = package.category().unwrap_or("");
    let matches = |needles: &[&str]| needles.iter().any(|n| category.contains(n));

    if matches(&["memo", "メモ", "note"]) {
        return IconKind::Notepad;
    }
    if matches(&["calendar", "カレンダー"]) {
        return IconKind::Calendar;
    }
    if matches(&["file", "ファイル"]) {
        return IconKind::Folder;
    }
    if matches(&["calc", "電卓"]) {
        return IconKind::Calculator;
    }
    if matches(&["setting", "設定"]) {
        return IconKind::Gear;
    }
    if matches(&["music", "mml", "音楽"]) {
        return IconKind::Music;
    }
    if matches(&["game", "ming", "ゲーム"]) {
        return IconKind::Game;
    }
    if matches(&["term", "echo", "input", "端末"]) {
        return IconKind::Terminal;
    }

    let mut hash = 0u8;
    for byte in id.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte);
    }
    ICON_KINDS[(hash as usize) % ICON_KINDS.len()]
}

fn draw_package_icon(
    canvas: &mut Canvas<'_>,
    rect: crate::hal::Rect,
    package: &PackageInfo,
    palette: &ShellPalette,
    selected: bool,
) {
    if let Some(icon) = package.icon() {
        draw_asset_icon(canvas, rect, icon, package.shell_icon(), palette, selected);
        return;
    }

    match icon_kind_for(package) {
        IconKind::Notepad => draw_notepad_icon(canvas, rect),
        IconKind::Calendar => draw_calendar_icon(canvas, rect),
        IconKind::Folder => draw_folder_icon(canvas, rect),
        IconKind::Calculator => draw_calculator_icon(canvas, rect),
        IconKind::Gear => draw_gear_icon(canvas, rect),
        IconKind::Music => draw_music_icon(canvas, rect),
        IconKind::Game => draw_game_icon(canvas, rect),
        IconKind::Terminal => draw_terminal_icon(canvas, rect),
    }
}

/// Draw a package-provided 40x40 monochrome icon. Consecutive set pixels are
/// coalesced into horizontal runs so an icon costs at most 40 fills instead of
/// 1,600 individual pixel writes.
fn draw_asset_icon(
    canvas: &mut Canvas<'_>,
    rect: crate::hal::Rect,
    icon: &crate::package::PackageIcon,
    theme: Option<PackageIconTheme>,
    palette: &ShellPalette,
    selected: bool,
) {
    let bg = if let Some(theme) = theme {
        Rgb565(theme.background)
    } else if selected {
        palette.selected_bg
    } else {
        palette.icon_bg
    };
    let ink = if let Some(theme) = theme {
        Rgb565(theme.primary)
    } else if selected {
        palette.selected_fg
    } else {
        palette.icon_fg
    };
    let shadow = if let Some(theme) = theme {
        Rgb565(theme.shadow)
    } else {
        palette.icon_shadow
    };
    canvas.fill_rect(rect, bg);

    // A one-pixel colored offset gives monochrome KICON1 artwork depth without
    // requiring a new package format. The source bitmap remains the silhouette.
    draw_asset_icon_mask(canvas, rect, icon, 1, 1, shadow);
    draw_asset_icon_mask(canvas, rect, icon, 0, 0, ink);

    if let Some(theme) = theme {
        icon_fill(canvas, rect, 28, 28, 5, 5, Rgb565(theme.accent));
        icon_fill(canvas, rect, 29, 29, 3, 3, Rgb565(theme.highlight));
    }
}

fn draw_asset_icon_mask(
    canvas: &mut Canvas<'_>,
    rect: crate::hal::Rect,
    icon: &crate::package::PackageIcon,
    offset_x: i32,
    offset_y: i32,
    color: Rgb565,
) {
    for y in 0..40 {
        if y as i32 + offset_y >= 40 {
            continue;
        }
        let mut x = 0;
        while x < 40 {
            while x < 40 && !icon.pixel(x, y) {
                x += 1;
            }
            let start = x;
            while x < 40 && icon.pixel(x, y) {
                x += 1;
            }
            if x > start {
                let draw_start = start as i32 + offset_x;
                let draw_end = (x as i32 + offset_x).min(40);
                if draw_start < 40 && draw_end > draw_start {
                    icon_fill(
                        canvas,
                        rect,
                        draw_start,
                        y as i32 + offset_y,
                        draw_end - draw_start,
                        1,
                        color,
                    );
                }
            }
        }
    }
}

/// Fill helper using rect-relative coordinates.
fn icon_fill(
    canvas: &mut Canvas<'_>,
    r: crate::hal::Rect,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    c: Rgb565,
) {
    canvas.fill_rect(
        crate::hal::Rect {
            x: r.x + x,
            y: r.y + y,
            w,
            h,
        },
        c,
    );
}

fn draw_notepad_icon(canvas: &mut Canvas<'_>, r: crate::hal::Rect) {
    let paper = Rgb565::from_rgb8(248, 248, 244);
    let edge = Rgb565::from_rgb8(120, 128, 140);
    let line = Rgb565::from_rgb8(60, 110, 200);
    let ring = Rgb565::from_rgb8(90, 96, 110);
    icon_fill(canvas, r, 8, 4, 24, 1, edge);
    icon_fill(canvas, r, 8, 33, 24, 1, edge);
    icon_fill(canvas, r, 8, 4, 1, 30, edge);
    icon_fill(canvas, r, 31, 4, 1, 30, edge);
    icon_fill(canvas, r, 9, 5, 22, 28, paper);
    for i in 0..3 {
        icon_fill(canvas, r, 12, 12 + i * 6, 16, 2, line);
    }
    for k in 0..3 {
        icon_fill(canvas, r, 12 + k * 7, 2, 3, 5, ring);
    }
}

fn draw_calendar_icon(canvas: &mut Canvas<'_>, r: crate::hal::Rect) {
    let body = Rgb565::from_rgb8(250, 250, 248);
    let header = Rgb565::from_rgb8(210, 70, 60);
    let edge = Rgb565::from_rgb8(150, 90, 80);
    let dot = Rgb565::from_rgb8(90, 100, 120);
    let ring = Rgb565::from_rgb8(70, 74, 84);
    icon_fill(canvas, r, 6, 8, 28, 26, body);
    icon_fill(canvas, r, 6, 6, 28, 8, header);
    icon_fill(canvas, r, 6, 33, 28, 1, edge);
    icon_fill(canvas, r, 12, 3, 3, 6, ring);
    icon_fill(canvas, r, 25, 3, 3, 6, ring);
    for row in 0..2 {
        for col in 0..3 {
            icon_fill(canvas, r, 10 + col * 8, 18 + row * 7, 4, 4, dot);
        }
    }
}

fn draw_folder_icon(canvas: &mut Canvas<'_>, r: crate::hal::Rect) {
    let folder = Rgb565::from_rgb8(235, 195, 75);
    let tab = Rgb565::from_rgb8(210, 170, 50);
    let edge = Rgb565::from_rgb8(180, 140, 40);
    icon_fill(canvas, r, 6, 9, 15, 6, tab);
    icon_fill(canvas, r, 6, 13, 28, 20, folder);
    icon_fill(canvas, r, 6, 13, 28, 1, edge);
    icon_fill(canvas, r, 6, 32, 28, 1, edge);
}

fn draw_calculator_icon(canvas: &mut Canvas<'_>, r: crate::hal::Rect) {
    let body = Rgb565::from_rgb8(60, 64, 72);
    let screen = Rgb565::from_rgb8(150, 210, 120);
    let key = Rgb565::from_rgb8(205, 210, 220);
    icon_fill(canvas, r, 8, 5, 24, 30, body);
    icon_fill(canvas, r, 11, 8, 18, 7, screen);
    for row in 0..3 {
        for col in 0..3 {
            icon_fill(canvas, r, 11 + col * 7, 19 + row * 5, 4, 3, key);
        }
    }
}

fn draw_gear_icon(canvas: &mut Canvas<'_>, r: crate::hal::Rect) {
    let gear = Rgb565::from_rgb8(160, 166, 174);
    let hole = Rgb565::from_rgb8(244, 244, 238);
    icon_fill(canvas, r, 11, 11, 18, 18, gear);
    // Teeth.
    icon_fill(canvas, r, 17, 5, 6, 6, gear);
    icon_fill(canvas, r, 17, 29, 6, 6, gear);
    icon_fill(canvas, r, 5, 17, 6, 6, gear);
    icon_fill(canvas, r, 29, 17, 6, 6, gear);
    // Hub.
    icon_fill(canvas, r, 16, 16, 8, 8, hole);
}

fn draw_music_icon(canvas: &mut Canvas<'_>, r: crate::hal::Rect) {
    let note = Rgb565::from_rgb8(40, 90, 200);
    icon_fill(canvas, r, 9, 24, 9, 7, note);
    icon_fill(canvas, r, 23, 20, 9, 7, note);
    icon_fill(canvas, r, 16, 8, 2, 19, note);
    icon_fill(canvas, r, 30, 4, 2, 19, note);
    icon_fill(canvas, r, 16, 6, 16, 4, note);
}

fn draw_game_icon(canvas: &mut Canvas<'_>, r: crate::hal::Rect) {
    let pad = Rgb565::from_rgb8(150, 156, 164);
    let dark = Rgb565::from_rgb8(50, 54, 62);
    let red = Rgb565::from_rgb8(210, 80, 70);
    let green = Rgb565::from_rgb8(110, 190, 110);
    icon_fill(canvas, r, 5, 13, 30, 16, pad);
    // D-pad.
    icon_fill(canvas, r, 9, 19, 9, 3, dark);
    icon_fill(canvas, r, 12, 16, 3, 9, dark);
    // Buttons.
    icon_fill(canvas, r, 25, 16, 4, 4, red);
    icon_fill(canvas, r, 29, 21, 4, 4, green);
}

fn draw_terminal_icon(canvas: &mut Canvas<'_>, r: crate::hal::Rect) {
    let screen = Rgb565::from_rgb8(28, 32, 40);
    let edge = Rgb565::from_rgb8(90, 96, 110);
    let glow = Rgb565::from_rgb8(120, 220, 130);
    icon_fill(canvas, r, 6, 7, 28, 26, edge);
    icon_fill(canvas, r, 7, 8, 26, 24, screen);
    // Prompt ">" and cursor.
    icon_fill(canvas, r, 11, 13, 5, 2, glow);
    icon_fill(canvas, r, 13, 15, 3, 2, glow);
    icon_fill(canvas, r, 11, 17, 5, 2, glow);
    icon_fill(canvas, r, 18, 23, 9, 2, glow);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hal::Buttons;

    fn sample_shell() -> ShellState {
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.one", "One").unwrap());
        packages.push(PackageInfo::new("dev.koto.two", "Two").unwrap());
        ShellState::new(packages)
    }

    #[test]
    fn empty_matches_new_with_empty_catalog() {
        assert_eq!(ShellState::empty(), ShellState::new(PackageList::new()));
    }

    #[test]
    fn reload_packages_fills_in_place_and_rebuilds_view() {
        let mut shell = ShellState::empty();
        assert_eq!(shell.visible_len(), 0);
        let status = shell.reload_packages(|packages| {
            assert!(packages.push(PackageInfo::new("dev.koto.one", "One").unwrap()));
            assert!(packages.push(PackageInfo::new("dev.koto.two", "Two").unwrap()));
            StorageStatus::Present
        });
        assert_eq!(status, StorageStatus::Present);
        assert_eq!(shell.visible_len(), 2);
        assert_eq!(shell.selected_package().unwrap().app_id(), "dev.koto.one");
        // The in-place reload must be equivalent to the by-value constructor.
        assert_eq!(shell, sample_shell());
        // A reload replaces the catalog (clears first) and resets the selection.
        shell.reload_packages(|packages| {
            assert!(packages.push(PackageInfo::new("dev.koto.three", "Three").unwrap()));
        });
        assert_eq!(shell.visible_len(), 1);
        assert_eq!(shell.selected_package().unwrap().app_id(), "dev.koto.three");
    }

    fn detailed_package() -> PackageInfo {
        let mut package = crate::package::PackageManifest::new(crate::package::ManifestFields {
            format: crate::package::KPA_MANIFEST_FORMAT,
            version: crate::package::KPA_MANIFEST_VERSION,
            app_id: "dev.koto.details",
            name: "Details",
            runtime: "kotoruntime-bytecode",
            entry: "bytecode/details.kbc",
            icon: None,
            shell_icon: None,
            fs_permission: Some("sandbox"),
            network_permission: Some(false),
            sram_work_bytes: Some(24_576),
            psram_cache_bytes: Some(32_768),
            description: Some("Detailed sample app."),
            category: Some("Tools"),
        })
        .unwrap()
        .package();
        package.set_save_data_present(true);
        package
    }

    #[test]
    fn down_selects_next_item() {
        let mut shell = sample_shell();
        let input = InputState {
            pressed: Buttons {
                down: true,
                ..Buttons::default()
            },
            ..InputState::default()
        };

        assert_eq!(shell.update(&input), ShellAction::None);
        assert_eq!(shell.selected_index(), 1);
    }

    #[test]
    fn confirm_launches_selected_package() {
        let mut shell = sample_shell();
        let input = InputState {
            pressed: Buttons {
                confirm: true,
                ..Buttons::default()
            },
            ..InputState::default()
        };

        match shell.update(&input) {
            ShellAction::Launch(package) => assert_eq!(package.name(), "One"),
            ShellAction::None => panic!("expected launch action"),
        }
    }

    #[test]
    fn cancel_toggles_detail_pane_without_launching() {
        let mut shell = sample_shell();
        assert!(shell.detail_pane_visible());

        let action = shell.update(&press(Buttons {
            cancel: true,
            ..Buttons::default()
        }));
        assert_eq!(action, ShellAction::None);
        assert!(!shell.detail_pane_visible());

        let action = shell.update(&press(Buttons {
            cancel: true,
            ..Buttons::default()
        }));
        assert_eq!(action, ShellAction::None);
        assert!(shell.detail_pane_visible());
    }

    #[test]
    fn toggle_reclamps_selection_into_the_new_layout() {
        // Fill the pane-shown last slot, then hide the pane and ensure the
        // selection stays valid for the wider grid.
        let mut shell = shell_with_packages(crate::package::MAX_PACKAGES);
        let last = shell.packages().len() - 1;
        while shell.selected_index() < last {
            shell.update(&press(Buttons {
                right: true,
                ..Buttons::default()
            }));
        }
        shell.toggle_detail_pane();
        assert!(!shell.detail_pane_visible());
        assert!(shell.selected_index() < shell.packages().len());
        assert!(shell.current_page() < shell.page_count());
    }

    #[test]
    fn unsupported_power_has_no_status_text() {
        let shell = sample_shell();

        assert_eq!(shell.power_state(), PowerState::Unsupported);
        assert_eq!(shell.status_text().as_str(), None);
    }

    #[test]
    fn low_battery_percent_is_marked_in_status_text() {
        let mut shell = sample_shell();

        shell.set_power_state(PowerState::percent(12, Some(3500)));

        assert_eq!(shell.status_text().as_str(), Some("LOW 12%"));
    }

    #[test]
    fn charging_and_unknown_power_have_status_text() {
        let mut shell = sample_shell();

        shell.set_power_state(PowerState::unknown());
        assert_eq!(shell.status_text().as_str(), Some("BAT ?"));

        shell.set_power_state(PowerState::charging(Some(88), None));
        assert_eq!(shell.status_text().as_str(), Some("CHG 88%"));
    }

    #[test]
    fn full_render_marks_entire_shell_surface() {
        let shell = sample_shell();
        let mut commands = RenderCommandList::<2>::new();

        shell.render_full(&mut commands).unwrap();

        let updates: std::vec::Vec<_> = commands.iter().map(|command| command.update).collect();
        assert_eq!(updates, [crate::render::RenderUpdate::Full]);
    }

    #[test]
    fn list_render_marks_header_and_each_package_tile() {
        let shell = sample_shell();
        let mut commands = RenderCommandList::<SHELL_LIST_COMMANDS>::new();

        shell.render_list(&mut commands).unwrap();

        let updates: std::vec::Vec<_> = commands.iter().map(|command| command.update).collect();
        let pane = shell.detail_pane_visible();
        assert_eq!(
            updates,
            std::vec![
                crate::render::RenderUpdate::Rect(shell_header_rect()),
                crate::render::RenderUpdate::Rect(shell_tile_rect(0, pane)),
                crate::render::RenderUpdate::Rect(shell_tile_rect(1, pane)),
                crate::render::RenderUpdate::Rect(shell_page_indicator_rect(pane)),
                crate::render::RenderUpdate::Rect(shell_pane_rect()),
                crate::render::RenderUpdate::Rect(shell_status_strip_rect()),
                crate::render::RenderUpdate::Rect(shell_command_bar_rect()),
            ]
        );
    }

    #[test]
    fn list_render_includes_all_regions_for_both_layout_modes() {
        let mut packages = PackageList::new();
        for index in 0..crate::package::MAX_PACKAGES {
            let app_id = std::format!("dev.koto.app{index}");
            packages.push(PackageInfo::new(&app_id, "App").unwrap());
        }
        let mut shell = ShellState::new(packages);

        for pane in [true, false] {
            shell.set_detail_pane_visible(pane);
            let mut commands = RenderCommandList::<SHELL_LIST_COMMANDS>::new();

            // No InvalidRect even though there are more packages than visible rows.
            shell.render_list(&mut commands).unwrap();

            // header + visible tiles + page indicator + (pane) + status + command.
            let expected = 1 + shell_visible_items(pane) + 1 + usize::from(pane) + 2;
            assert_eq!(commands.len(), expected);
            assert!(expected <= SHELL_LIST_COMMANDS);
        }
    }

    /// A valid but glyph-less `.kfont` blob, enough to drive `paint` (text draws
    /// nothing, backgrounds still fill).
    fn empty_font_bytes() -> std::vec::Vec<u8> {
        let mut data = std::vec::Vec::new();
        data.extend_from_slice(b"KFNT");
        data.extend_from_slice(&1u16.to_le_bytes()); // version
        data.extend_from_slice(&0u16.to_le_bytes()); // flags
        data.push(13); // cell_h
        data.push(11); // ascent
        data.push(6); // half_w
        data.push(12); // full_w
        data.extend_from_slice(&0u32.to_le_bytes()); // glyph_count
        data
    }

    #[test]
    fn paint_highlights_only_the_selected_tile() {
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.one", "One").unwrap());
        packages.push(PackageInfo::new("dev.koto.two", "Two").unwrap());
        let mut shell = ShellState::new(packages);
        shell.update(&InputState {
            pressed: Buttons {
                down: true,
                ..Buttons::default()
            },
            ..InputState::default()
        });
        assert_eq!(shell.selected_index(), 1);
        for _ in 0..SHELL_SELECTION_FEEDBACK_FRAMES {
            shell.advance_feedback();
        }

        let font_bytes = empty_font_bytes();
        let font = crate::font::BitmapFont::from_bytes(&font_bytes).unwrap();
        let w = SHELL_SURFACE.width;
        let h = SHELL_SURFACE.height;
        let mut buf = std::vec![0u8; w as usize * h as usize * 2];
        let mut canvas = Canvas::new(&mut buf, w, h).unwrap();

        shell.paint(&mut canvas, &font);

        // Sample near each tile's top-left, clear of the border, icon, and label.
        let palette = ShellPalette::DEFAULT;
        let tile0 = shell_tile_rect(0, shell.detail_pane_visible());
        let tile1 = shell_tile_rect(1, shell.detail_pane_visible());
        let sample_y = (SHELL_HEADER_HEIGHT + 4) as u16;

        assert_eq!(
            pixel(&buf, w, (tile0.x + 3) as u16, sample_y),
            palette.row_bg.0
        );
        assert_eq!(
            pixel(&buf, w, (tile1.x + 3) as u16, sample_y),
            palette.selected_bg.0
        );
    }

    #[test]
    fn system_view_paints_overlay_body() {
        let mut shell = sample_shell();
        // Hide the detail pane so the launcher's right edge is not itself pane_bg.
        shell.set_detail_pane_visible(false);
        shell.set_memory_status(MemoryStatus {
            sram_total: 264 * 1024,
            sram_free_min: Some(7620),
            psram_present: true,
            psram_total: 8 * 1024 * 1024,
            code_window_slots: 2,
            ..MemoryStatus::unknown()
        });

        let font_bytes = empty_font_bytes();
        let font = crate::font::BitmapFont::from_bytes(&font_bytes).unwrap();
        let w = SHELL_SURFACE.width;
        let h = SHELL_SURFACE.height;
        let palette = ShellPalette::DEFAULT;

        // A right-edge point in the body band, clear of the left-aligned rows.
        let sx = w - 10;
        let sy = (SHELL_HEADER_HEIGHT + 40) as u16;

        // Launcher: this point is not the overlay panel fill.
        let mut launcher = std::vec![0u8; w as usize * h as usize * 2];
        shell.paint(&mut Canvas::new(&mut launcher, w, h).unwrap(), &font);
        assert_ne!(pixel(&launcher, w, sx, sy), palette.pane_bg.0);

        // Overlay: the same point is the panel background, proving the system
        // view replaced the launcher body.
        shell.set_system_view_visible(true);
        let mut overlay = std::vec![0u8; w as usize * h as usize * 2];
        shell.paint(&mut Canvas::new(&mut overlay, w, h).unwrap(), &font);
        assert_eq!(pixel(&overlay, w, sx, sy), palette.pane_bg.0);
    }

    #[test]
    fn paint_rect_matches_full_paint_inside_clip_and_leaves_outside_untouched() {
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.one", "One").unwrap());
        packages.push(PackageInfo::new("dev.koto.two", "Two").unwrap());
        let mut shell = ShellState::new(packages);
        // Hide the pane to exercise the device's full-width launcher layout.
        shell.set_detail_pane_visible(false);
        shell.update(&InputState {
            pressed: Buttons {
                right: true,
                ..Buttons::default()
            },
            ..InputState::default()
        });
        // Skip the bounded selection animation, matching the device redraw path.
        for _ in 0..SHELL_SELECTION_FEEDBACK_FRAMES {
            shell.advance_feedback();
        }

        let font_bytes = empty_font_bytes();
        let font = crate::font::BitmapFont::from_bytes(&font_bytes).unwrap();
        let w = SHELL_SURFACE.width;
        let h = SHELL_SURFACE.height;

        let mut full = std::vec![0u8; w as usize * h as usize * 2];
        shell.paint(&mut Canvas::new(&mut full, w, h).unwrap(), &font);

        // Repaint only the newly selected tile over a sentinel-filled buffer.
        const SENTINEL: u16 = 0x1234;
        let mut bounded = std::vec![0u8; w as usize * h as usize * 2];
        for chunk in bounded.chunks_exact_mut(2) {
            chunk.copy_from_slice(&SENTINEL.to_le_bytes());
        }
        let clip = shell_tile_rect(1, false);
        shell.paint_rect(&mut Canvas::new(&mut bounded, w, h).unwrap(), &font, clip);

        for y in 0..h {
            for x in 0..w {
                let inside = rects_intersect(
                    clip,
                    crate::hal::Rect {
                        x: i32::from(x),
                        y: i32::from(y),
                        w: 1,
                        h: 1,
                    },
                );
                let bounded_px = pixel(&bounded, w, x, y);
                if inside {
                    assert_eq!(
                        bounded_px,
                        pixel(&full, w, x, y),
                        "clip pixel ({x},{y}) must match full paint"
                    );
                } else {
                    assert_eq!(
                        bounded_px, SENTINEL,
                        "pixel ({x},{y}) outside clip must be untouched"
                    );
                }
            }
        }
    }

    #[test]
    fn icon_kind_is_assigned_by_category() {
        // Category drives the icon even when the app id is opaque.
        let mut calendar = crate::package::PackageManifest::new(crate::package::ManifestFields {
            format: crate::package::KPA_MANIFEST_FORMAT,
            version: crate::package::KPA_MANIFEST_VERSION,
            app_id: "dev.koto.app.x",
            name: "X",
            runtime: "kotoruntime-bytecode",
            entry: "bytecode/x.kbc",
            icon: None,
            shell_icon: None,
            fs_permission: None,
            network_permission: None,
            sram_work_bytes: None,
            psram_cache_bytes: None,
            description: None,
            category: Some("カレンダー"),
        })
        .unwrap()
        .package();
        calendar.set_save_data_present(false);
        assert_eq!(icon_kind_for(&calendar), IconKind::Calendar);
    }

    #[test]
    fn icon_rich_page_paints_distinct_colored_icons() {
        // Two apps with different icon kinds should paint different icon pixels.
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.memo", "Memo").unwrap());
        packages.push(PackageInfo::new("dev.koto.samples.file-note", "File").unwrap());
        let shell = ShellState::new(packages);
        assert_ne!(
            icon_kind_for(shell.packages().get(0).unwrap()),
            IconKind::Folder
        );

        let font_bytes = empty_font_bytes();
        let font = crate::font::BitmapFont::from_bytes(&font_bytes).unwrap();
        let w = SHELL_SURFACE.width;
        let h = SHELL_SURFACE.height;
        let mut buf = std::vec![0u8; w as usize * h as usize * 2];
        let mut canvas = Canvas::new(&mut buf, w, h).unwrap();

        shell.paint(&mut canvas, &font);

        // Both icon areas contain non-background pixels.
        let palette = ShellPalette::DEFAULT;
        let icon0 = icon_rect(shell_tile_rect(0, true));
        let icon1 = icon_rect(shell_tile_rect(1, true));
        let sample0 = pixel(&buf, w, (icon0.x + 20) as u16, (icon0.y + 20) as u16);
        let sample1 = pixel(&buf, w, (icon1.x + 20) as u16, (icon1.y + 20) as u16);
        assert_ne!(sample0, palette.background.0);
        assert_ne!(sample1, palette.background.0);
    }

    #[test]
    fn detail_pane_paints_within_its_region() {
        let mut packages = PackageList::new();
        packages.push(detailed_package());
        let shell = ShellState::new(packages);
        assert!(shell.detail_pane_visible());

        let font_bytes = empty_font_bytes();
        let font = crate::font::BitmapFont::from_bytes(&font_bytes).unwrap();
        let w = SHELL_SURFACE.width;
        let h = SHELL_SURFACE.height;
        let mut buf = std::vec![0u8; w as usize * h as usize * 2];
        let mut canvas = Canvas::new(&mut buf, w, h).unwrap();

        shell.paint(&mut canvas, &font);

        // The pane fills its region with the pane background.
        let palette = ShellPalette::DEFAULT;
        let pane = shell_pane_rect();
        assert_eq!(
            pixel(&buf, w, (pane.x + pane.w / 2) as u16, (pane.y + 30) as u16),
            palette.pane_bg.0
        );
        // The header band still paints on top.
        assert_eq!(pixel(&buf, w, 0, 0), palette.header_bg.0);
    }

    fn pixel(buf: &[u8], width: u16, x: u16, y: u16) -> u16 {
        let i = (y as usize * width as usize + x as usize) * 2;
        u16::from_le_bytes([buf[i], buf[i + 1]])
    }

    #[test]
    fn selection_change_marks_previous_and_current_tiles() {
        let mut shell = sample_shell();
        let previous = shell.selected_index();
        shell.update(&InputState {
            pressed: Buttons {
                down: true,
                ..Buttons::default()
            },
            ..InputState::default()
        });

        let mut commands = RenderCommandList::<SHELL_LIST_COMMANDS>::new();
        shell
            .render_selection_change(previous, &mut commands)
            .unwrap();

        let updates: std::vec::Vec<_> = commands.iter().map(|command| command.update).collect();
        let pane = shell.detail_pane_visible();
        assert_eq!(
            updates,
            std::vec![
                crate::render::RenderUpdate::Rect(shell_tile_rect(previous, pane)),
                crate::render::RenderUpdate::Rect(shell_tile_rect(shell.selected_index(), pane)),
                crate::render::RenderUpdate::Rect(shell_pane_rect()),
                crate::render::RenderUpdate::Rect(shell_status_strip_rect()),
            ]
        );
    }

    #[test]
    fn toggling_detail_pane_changes_grid_geometry() {
        let mut shell = sample_shell();
        assert!(shell.detail_pane_visible());
        let with_pane = shell_tile_rect(0, true);

        shell.toggle_detail_pane();
        assert!(!shell.detail_pane_visible());
        let without_pane = shell_tile_rect(0, false);

        // The launcher reclaims width when the pane is hidden.
        assert!(without_pane.w > with_pane.w);
    }

    fn shell_with_packages(count: usize) -> ShellState {
        let mut packages = PackageList::new();
        for index in 0..count {
            let app_id = std::format!("dev.koto.app{index}");
            packages.push(PackageInfo::new(&app_id, "App").unwrap());
        }
        ShellState::new(packages)
    }

    fn press(buttons: Buttons) -> InputState {
        InputState {
            pressed: buttons,
            ..InputState::default()
        }
    }

    #[test]
    fn navigation_moves_by_grid_columns() {
        let mut shell = shell_with_packages(12);
        let columns = shell_grid_columns(shell.detail_pane_visible());

        shell.update(&press(Buttons {
            down: true,
            ..Buttons::default()
        }));
        assert_eq!(shell.selected_index(), columns);

        shell.update(&press(Buttons {
            right: true,
            ..Buttons::default()
        }));
        assert_eq!(shell.selected_index(), columns + 1);

        shell.update(&press(Buttons {
            left: true,
            ..Buttons::default()
        }));
        assert_eq!(shell.selected_index(), columns);

        shell.update(&press(Buttons {
            up: true,
            ..Buttons::default()
        }));
        assert_eq!(shell.selected_index(), 0);
    }

    #[test]
    fn down_past_page_boundary_advances_page() {
        let mut shell = shell_with_packages(crate::package::MAX_PACKAGES);
        assert_eq!(shell.current_page(), 0);

        let per = shell.items_per_page();
        while shell.selected_index() < per {
            shell.update(&press(Buttons {
                down: true,
                ..Buttons::default()
            }));
        }
        assert!(shell.current_page() >= 1);
        assert_eq!(shell.page_feedback_frames(), SHELL_PAGE_FEEDBACK_FRAMES);
        assert_eq!(shell.take_pending_sound(), Some(ShellSound::Navigation));
    }

    #[test]
    fn navigation_feedback_is_bounded_and_expires() {
        let mut shell = shell_with_packages(2);
        let pane = shell.detail_pane_visible();
        let from = shell_tile_rect(0, pane);
        let to = shell_tile_rect(1, pane);
        shell.update(&press(Buttons {
            right: true,
            ..Buttons::default()
        }));

        assert_eq!(
            shell.selection_feedback_frames(),
            SHELL_SELECTION_FEEDBACK_FRAMES
        );
        assert_eq!(shell.take_pending_sound(), Some(ShellSound::Navigation));
        assert_eq!(shell.selection_cursor_rect(pane), from);
        for _ in 0..3 {
            shell.advance_feedback();
        }
        let halfway = shell.selection_cursor_rect(pane);
        assert!(halfway.x > from.x);
        assert!(halfway.x < to.x);
        for _ in 0..SHELL_SELECTION_FEEDBACK_FRAMES {
            shell.advance_feedback();
        }
        assert_eq!(shell.selection_feedback_frames(), 0);
        assert_eq!(shell.selection_cursor_rect(pane), to);
        assert_eq!(shell.take_pending_sound(), None);
    }

    #[test]
    fn confirm_and_cancel_emit_distinct_cues_without_changing_navigation() {
        let mut shell = sample_shell();
        let selected = shell.selected_index();
        assert!(matches!(
            shell.update(&press(Buttons {
                confirm: true,
                ..Buttons::default()
            })),
            ShellAction::Launch(_)
        ));
        assert_eq!(shell.take_pending_sound(), Some(ShellSound::Confirm));
        assert_eq!(shell.selected_index(), selected);

        let pane = shell.detail_pane_visible();
        assert_eq!(
            shell.update(&press(Buttons {
                cancel: true,
                ..Buttons::default()
            })),
            ShellAction::None
        );
        assert_eq!(shell.take_pending_sound(), Some(ShellSound::Cancel));
        assert_ne!(shell.detail_pane_visible(), pane);
    }

    #[test]
    fn detail_pane_slides_from_and_to_the_right_edge() {
        let mut shell = sample_shell();
        shell.set_detail_pane_visible(false);
        shell.toggle_detail_pane();

        assert_eq!(shell.pane_transition_frames(), SHELL_PANE_TRANSITION_FRAMES);
        assert_eq!(shell.animated_pane_rect().x, i32::from(SHELL_SURFACE.width));
        for _ in 0..4 {
            shell.advance_feedback();
        }
        let middle = shell.animated_pane_rect().x;
        assert!(middle > shell_pane_rect().x);
        assert!(middle < i32::from(SHELL_SURFACE.width));
        for _ in 0..SHELL_PANE_TRANSITION_FRAMES {
            shell.advance_feedback();
        }
        assert_eq!(shell.animated_pane_rect(), shell_pane_rect());

        shell.toggle_detail_pane();
        assert_eq!(shell.animated_pane_rect(), shell_pane_rect());
        for _ in 0..SHELL_PANE_TRANSITION_FRAMES {
            shell.advance_feedback();
        }
        assert_eq!(shell.pane_transition_frames(), 0);
    }

    #[test]
    fn page_change_slides_old_and_new_pages_in_navigation_direction() {
        let mut shell = shell_with_packages(crate::package::MAX_PACKAGES);
        let pane = shell.detail_pane_visible();
        while shell.current_page() == 0 {
            shell.update(&press(Buttons {
                down: true,
                ..Buttons::default()
            }));
        }

        let (from_page, from_dx, current_dx) = shell.page_slide_offsets(pane).unwrap();
        assert_eq!(from_page, 0);
        assert_eq!(from_dx, 0);
        assert_eq!(current_dx, shell_grid_area_width(pane));
        for _ in 0..4 {
            shell.advance_feedback();
        }
        let (_, from_dx, current_dx) = shell.page_slide_offsets(pane).unwrap();
        assert!(from_dx < 0);
        assert!(current_dx > 0);
        assert!(current_dx < shell_grid_area_width(pane));
        for _ in 0..SHELL_PAGE_FEEDBACK_FRAMES {
            shell.advance_feedback();
        }
        assert_eq!(shell.page_slide_offsets(pane), None);
    }

    #[test]
    fn page_count_reflects_layout_mode() {
        let mut shell = shell_with_packages(crate::package::MAX_PACKAGES);

        shell.set_detail_pane_visible(true);
        let pages_with_pane = shell.page_count();
        shell.set_detail_pane_visible(false);
        let pages_without_pane = shell.page_count();

        assert!(pages_with_pane >= 1);
        // The pane-hidden grid is wider, so it needs no more pages.
        assert!(pages_without_pane <= pages_with_pane);
    }

    #[test]
    fn selection_change_across_pages_repaints_grid_area() {
        let mut shell = shell_with_packages(crate::package::MAX_PACKAGES);
        let pane = shell.detail_pane_visible();
        let previous = shell.selected_index();

        while shell.current_page() == 0 {
            shell.update(&press(Buttons {
                down: true,
                ..Buttons::default()
            }));
        }

        let mut commands = RenderCommandList::<SHELL_LIST_COMMANDS>::new();
        shell
            .render_selection_change(previous, &mut commands)
            .unwrap();

        let updates: std::vec::Vec<_> = commands.iter().map(|command| command.update).collect();
        assert!(
            updates.contains(&crate::render::RenderUpdate::Rect(shell_grid_area_rect(
                pane
            )))
        );
    }

    #[test]
    fn clock_text_formats_fixed_timestamp_with_placeholder() {
        let mut shell = sample_shell();
        assert_eq!(shell.clock_text().as_str(), Some("----/--/-- --:--"));

        shell.set_clock(ShellClock {
            year: 2025,
            month: 5,
            day: 18,
            hour: 10,
            minute: 42,
        });
        assert_eq!(shell.clock_text().as_str(), Some("2025/05/18 10:42"));

        shell.clear_clock();
        assert_eq!(shell.clock_text().as_str(), Some("----/--/-- --:--"));
    }

    #[test]
    fn battery_indicators_cover_each_power_state() {
        let mut shell = sample_shell();
        // Unsupported (default).
        assert_eq!(shell.battery_percent(), None);
        assert!(!shell.battery_is_low());
        assert!(!shell.battery_is_charging());

        // Normal.
        shell.set_power_state(PowerState::percent(92, None));
        assert_eq!(shell.battery_percent(), Some(92));
        assert!(!shell.battery_is_low());

        // Low.
        shell.set_power_state(PowerState::percent(10, None));
        assert!(shell.battery_is_low());

        // Charging.
        shell.set_power_state(PowerState::charging(Some(50), None));
        assert!(shell.battery_is_charging());
        assert_eq!(shell.battery_percent(), Some(50));

        // Unknown.
        shell.set_power_state(PowerState::unknown());
        assert_eq!(shell.battery_percent(), None);
    }

    #[test]
    fn storage_and_save_indicators_have_text() {
        let mut shell = sample_shell();
        assert_eq!(shell.storage_indicator_text(), "SD?");
        assert_eq!(shell.save_indicator_text(), None);

        shell.set_storage_status(StorageStatus::Present);
        shell.set_save_status(SaveStatus::Saved);
        assert_eq!(shell.storage_indicator_text(), "SD");
        assert_eq!(shell.save_indicator_text(), Some("保存済"));

        shell.set_storage_status(StorageStatus::Absent);
        shell.set_save_status(SaveStatus::Unsaved);
        assert_eq!(shell.storage_indicator_text(), "SD×");
        assert_eq!(shell.save_indicator_text(), Some("未保存"));
    }

    #[test]
    fn command_bar_reflects_state_and_availability() {
        let mut shell = sample_shell();
        let bar = shell.command_bar();

        // Launch is available when packages exist.
        assert_eq!(bar[0].label, "開く");
        assert!(bar[0].enabled);

        // Favorite, sort, and category are now actionable.
        assert_eq!(bar[1].label, "お気に入り");
        assert!(bar[1].enabled);
        assert_eq!(bar[2].label, "並替");
        assert!(bar[2].enabled);
        assert_eq!(bar[3].label, "カテゴリ");
        assert!(bar[3].enabled);

        // The detail-pane toggle reports its current state.
        assert_eq!(bar[4].label, "詳細");
        assert_eq!(bar[4].state, Some("ON"));
        shell.toggle_detail_pane();
        assert_eq!(shell.command_bar()[4].state, Some("OFF"));

        // The system status view is reachable from the launcher (KOTO-0182).
        assert_eq!(bar[5].key, "F5");
        assert_eq!(bar[5].label, "システム");
        assert!(bar[5].enabled);
    }

    #[test]
    fn system_view_toggle_and_dismiss() {
        let mut shell = sample_shell();
        assert!(!shell.system_view_visible());

        // F5 route (host-driven, like sort/category) opens the overlay.
        shell.toggle_system_view();
        assert!(shell.system_view_visible());

        // Navigation is captured while the overlay is open: selection is frozen.
        let selected = shell.selected_index();
        shell.update(&press(Buttons {
            down: true,
            ..Buttons::default()
        }));
        assert!(shell.system_view_visible());
        assert_eq!(shell.selected_index(), selected);

        // Confirm dismisses back to the launcher without launching an app.
        assert_eq!(
            shell.update(&press(Buttons {
                confirm: true,
                ..Buttons::default()
            })),
            ShellAction::None,
        );
        assert!(!shell.system_view_visible());

        // Cancel also dismisses (rather than toggling the detail pane).
        shell.toggle_system_view();
        shell.update(&press(Buttons {
            cancel: true,
            ..Buttons::default()
        }));
        assert!(!shell.system_view_visible());
    }

    #[test]
    fn memory_status_round_trips_and_defaults_unknown() {
        let mut shell = sample_shell();
        assert_eq!(shell.memory_status(), MemoryStatus::unknown());

        let status = MemoryStatus {
            sram_total: 264 * 1024,
            sram_free_min: Some(7620),
            psram_present: true,
            psram_total: 8 * 1024 * 1024,
            code_window_slots: 2,
            ..MemoryStatus::unknown()
        };
        shell.set_memory_status(status);
        assert_eq!(shell.memory_status(), status);
    }

    #[test]
    fn kib_formatter_matches_free_min() {
        let mut text = ShellDetailText::empty();
        text.push_kib(7620);
        assert_eq!(text.as_str(), "7.4KiB");

        let mut total = ShellDetailText::empty();
        total.push_kib(264 * 1024);
        assert_eq!(total.as_str(), "264.0KiB");

        assert_eq!(mem_kib_text(None).as_str(), "----");

        let mut psram = ShellDetailText::empty();
        psram.push_mib(8 * 1024 * 1024);
        assert_eq!(psram.as_str(), "8MiB");
    }

    #[test]
    fn command_bar_launch_disabled_without_packages() {
        let shell = ShellState::new(PackageList::new());
        assert_eq!(shell.command_bar()[0].label, "開く");
        assert!(!shell.command_bar()[0].enabled);
    }

    fn categorized(app_id: &str, name: &str, category: &str) -> PackageInfo {
        crate::package::PackageManifest::new(crate::package::ManifestFields {
            format: crate::package::KPA_MANIFEST_FORMAT,
            version: crate::package::KPA_MANIFEST_VERSION,
            app_id,
            name,
            runtime: "kotoruntime-bytecode",
            entry: "bytecode/x.kbc",
            icon: None,
            shell_icon: None,
            fs_permission: None,
            network_permission: None,
            sram_work_bytes: None,
            psram_cache_bytes: None,
            description: None,
            category: Some(category),
        })
        .unwrap()
        .package()
    }

    fn view_names(shell: &ShellState) -> std::vec::Vec<std::string::String> {
        (0..shell.visible_len())
            .map(|v| {
                shell
                    .packages()
                    .get(shell.package_index_at(v).unwrap())
                    .unwrap()
                    .name()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn toggle_favorite_marks_selected_package() {
        let mut shell = sample_shell();
        assert!(!shell.selected_is_favorite());
        shell.toggle_selected_favorite();
        assert!(shell.selected_is_favorite());
        assert!(shell.selected_package().unwrap().is_favorite());
        shell.toggle_selected_favorite();
        assert!(!shell.selected_is_favorite());
    }

    #[test]
    fn name_sort_orders_packages_alphabetically() {
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.c", "C").unwrap());
        packages.push(PackageInfo::new("dev.koto.a", "A").unwrap());
        packages.push(PackageInfo::new("dev.koto.b", "B").unwrap());
        let mut shell = ShellState::new(packages);

        shell.set_sort_mode(SortMode::Name);
        assert_eq!(view_names(&shell), ["A", "B", "C"]);
    }

    #[test]
    fn favorite_sort_lists_favorites_first() {
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.a", "A").unwrap());
        packages.push(PackageInfo::new("dev.koto.b", "B").unwrap());
        let mut shell = ShellState::new(packages);

        assert!(shell.set_favorite_by_app_id("dev.koto.b", true));
        shell.set_sort_mode(SortMode::Favorite);
        assert_eq!(view_names(&shell), ["B", "A"]);
    }

    #[test]
    fn category_cycle_filters_visible_packages() {
        let mut packages = PackageList::new();
        packages.push(categorized("dev.koto.a", "A", "Tools"));
        packages.push(categorized("dev.koto.b", "B", "Games"));
        packages.push(categorized("dev.koto.c", "C", "Tools"));
        let mut shell = ShellState::new(packages);

        assert_eq!(shell.visible_len(), 3);
        assert_eq!(shell.category_filter(), None);

        shell.cycle_category();
        assert_eq!(shell.category_filter(), Some("Tools"));
        assert_eq!(shell.visible_len(), 2);

        shell.cycle_category();
        assert_eq!(shell.category_filter(), Some("Games"));
        assert_eq!(shell.visible_len(), 1);

        shell.cycle_category();
        assert_eq!(shell.category_filter(), None);
        assert_eq!(shell.visible_len(), 3);
    }
}
