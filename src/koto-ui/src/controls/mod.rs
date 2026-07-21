mod button;
mod checkbox;
mod dialog;
mod label;
mod list;
mod panel;
mod text_field;

pub use button::Button;
pub use checkbox::Checkbox;
pub use dialog::{
    Dialog, DialogAction, DialogError, DialogResult, DialogResultKind,
    DEFAULT_DIALOG_ACTION_CAPACITY, DEFAULT_DIALOG_CHILD_CAPACITY,
};
pub use label::Label;
pub use list::{List, ListError, ListModel, ListRow};
pub use panel::{Insets, LayoutError, Panel, PanelLayout};
pub use text_field::{BufferError, ImeComposition, TextField, Utf8Buffer};

use crate::{ControlStyle, PaintError, Painter, Theme, UiRect, VisualState};

fn control_style(theme: &Theme, state: VisualState) -> ControlStyle {
    match state {
        VisualState::Normal => theme.normal,
        VisualState::Focused => theme.focused,
        VisualState::Pressed => theme.pressed,
        VisualState::Disabled => theme.disabled,
    }
}

fn clipped(bounds: UiRect, clip: UiRect) -> Option<UiRect> {
    bounds.intersection(clip)
}

fn paint_frame(
    painter: &mut impl Painter,
    clip: UiRect,
    bounds: UiRect,
    style: ControlStyle,
    theme: &Theme,
) -> Result<(), PaintError> {
    painter.fill_rect(clip, bounds, style.background)?;
    painter.stroke_rect(clip, bounds, style.border, theme.border_width)
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;
    use crate::{
        Button, Checkbox, EventPhase, GlyphRun, Label, ResponseKind, Rgb565, TextAlign,
        TextMetrics, TextRun, UiAction, UiContext, UiEvent, UiResponse, WidgetId,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum PaintOp {
        Fill(UiRect, UiRect, Rgb565),
        Stroke(UiRect, UiRect, Rgb565, u8),
        Text(UiRect, UiRect, std::string::String, Rgb565, TextAlign),
        Glyphs(UiRect, UiRect, std::vec::Vec<u16>, Rgb565, i16),
        Focus(UiRect, UiRect, Rgb565, u8),
    }

    #[derive(Default)]
    struct RecordingPainter {
        ops: std::vec::Vec<PaintOp>,
    }

    impl TextMetrics for RecordingPainter {
        fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
            Ok(text
                .chars()
                .map(|ch| if ch.is_ascii() { 6 } else { 12 })
                .sum())
        }
    }

    impl Painter for RecordingPainter {
        fn fill_rect(
            &mut self,
            clip: UiRect,
            rect: UiRect,
            color: Rgb565,
        ) -> Result<(), PaintError> {
            self.ops.push(PaintOp::Fill(clip, rect, color));
            Ok(())
        }

        fn stroke_rect(
            &mut self,
            clip: UiRect,
            rect: UiRect,
            color: Rgb565,
            width: u8,
        ) -> Result<(), PaintError> {
            self.ops.push(PaintOp::Stroke(clip, rect, color, width));
            Ok(())
        }

        fn draw_text(
            &mut self,
            clip: UiRect,
            bounds: UiRect,
            run: TextRun<'_>,
        ) -> Result<(), PaintError> {
            self.ops.push(PaintOp::Text(
                clip,
                bounds,
                run.text.into(),
                run.color,
                run.align,
            ));
            Ok(())
        }

        fn draw_glyphs(
            &mut self,
            clip: UiRect,
            bounds: UiRect,
            run: GlyphRun<'_>,
        ) -> Result<(), PaintError> {
            self.ops.push(PaintOp::Glyphs(
                clip,
                bounds,
                run.glyphs.into(),
                run.color,
                run.spacing,
            ));
            Ok(())
        }

        fn draw_focus_mark(
            &mut self,
            clip: UiRect,
            rect: UiRect,
            color: Rgb565,
            width: u8,
        ) -> Result<(), PaintError> {
            self.ops.push(PaintOp::Focus(clip, rect, color, width));
            Ok(())
        }
    }

    const ID: WidgetId = WidgetId::new(7);
    const BOUNDS: UiRect = UiRect::new(10, 20, 100, 20);
    const SURFACE: UiRect = UiRect::new(0, 0, 320, 320);

    fn context() -> UiContext<8> {
        UiContext::new(SURFACE, Theme::DARK)
    }

    #[test]
    fn label_alignment_and_clip_are_forwarded_deterministically() {
        for align in [TextAlign::Start, TextAlign::Center, TextAlign::End] {
            let label = Label::new(ID, BOUNDS, "長い borrowed label").with_alignment(align);
            let mut painter = RecordingPainter::default();
            let clip = UiRect::new(20, 20, 30, 20);
            label.paint(&mut painter, clip, &Theme::DARK).unwrap();
            assert_eq!(
                painter.ops,
                [PaintOp::Text(
                    clip,
                    BOUNDS,
                    "長い borrowed label".into(),
                    Theme::DARK.normal.foreground,
                    align,
                )]
            );
        }
    }

    #[test]
    fn label_changes_damage_bounds_and_idle_setters_do_not() {
        let mut label = Label::new(ID, BOUNDS, "old");
        let mut context = context();
        label.set_text("old", &mut context);
        label.set_alignment(TextAlign::Start, &mut context);
        assert!(!context.has_damage());
        label.set_text("new", &mut context);
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [BOUNDS]
        );
    }

    #[test]
    fn button_paints_every_state_in_stable_order() {
        let mut button = Button::new(ID, BOUNDS, "OK");
        let mut context = context();
        for state in [
            VisualState::Normal,
            VisualState::Focused,
            VisualState::Pressed,
            VisualState::Disabled,
        ] {
            button.set_enabled(state != VisualState::Disabled, &mut context);
            button.set_focused(
                matches!(state, VisualState::Focused | VisualState::Pressed),
                &mut context,
            );
            button.set_pressed(state == VisualState::Pressed, &mut context);
            let mut painter = RecordingPainter::default();
            button.paint(&mut painter, SURFACE, &Theme::DARK).unwrap();
            let expected_style = control_style(&Theme::DARK, state);
            let expected_len = if matches!(state, VisualState::Focused | VisualState::Pressed) {
                4
            } else {
                3
            };
            assert_eq!(painter.ops.len(), expected_len);
            assert!(matches!(painter.ops[0], PaintOp::Fill(..)));
            assert!(matches!(
                painter.ops[0],
                PaintOp::Fill(_, _, color) if color == expected_style.background
            ));
            assert!(matches!(painter.ops[1], PaintOp::Stroke(..)));
            assert!(matches!(
                painter.ops[1],
                PaintOp::Stroke(_, _, color, _) if color == expected_style.border
            ));
            assert!(matches!(painter.ops[2], PaintOp::Text(..)));
            assert!(matches!(
                painter.ops[2],
                PaintOp::Text(_, _, _, color, _) if color == expected_style.foreground
            ));
            if expected_len == 4 {
                assert!(matches!(painter.ops[3], PaintOp::Focus(..)));
            }
        }
    }

    #[test]
    fn button_activates_once_per_press_and_release_clears_visual() {
        let mut button = Button::new(ID, BOUNDS, "OK");
        let mut context = context();
        button.set_focused(true, &mut context);
        context.clear_damage();
        let pressed = UiEvent::pressed(UiAction::Activate);
        assert_eq!(
            button.handle_event(pressed, &mut context),
            Some(UiResponse::new(ID, ResponseKind::Activated))
        );
        assert_eq!(button.visual_state(), VisualState::Pressed);
        assert_eq!(button.handle_event(pressed, &mut context), None);
        assert_eq!(
            button.handle_event(UiEvent::repeated(UiAction::Activate), &mut context),
            None
        );
        assert_eq!(
            button.handle_event(UiEvent::released(UiAction::Activate), &mut context),
            None
        );
        assert_eq!(button.visual_state(), VisualState::Focused);
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [BOUNDS]
        );
    }

    #[test]
    fn disabled_button_ignores_activation_and_uses_semantic_label() {
        let mut button = Button::new(ID, BOUNDS, "×").with_semantic_label("Close");
        let mut context = context();
        button.set_focused(true, &mut context);
        button.set_enabled(false, &mut context);
        assert_eq!(button.semantic_label(), "Close");
        assert_eq!(
            button.handle_event(UiEvent::pressed(UiAction::Activate), &mut context),
            None
        );
        assert_eq!(button.visual_state(), VisualState::Disabled);
    }

    #[test]
    fn checkbox_toggles_once_and_paints_shape_not_only_color() {
        let mut checkbox = Checkbox::new(ID, BOUNDS, "Wi-Fi");
        let mut context = context();
        checkbox.set_focused(true, &mut context);
        context.clear_damage();
        assert_eq!(
            checkbox.handle_event(UiEvent::pressed(UiAction::Activate), &mut context),
            Some(UiResponse::new(ID, ResponseKind::ValueChanged(1)))
        );
        assert!(checkbox.is_checked());
        assert_eq!(
            checkbox.handle_event(UiEvent::repeated(UiAction::Activate), &mut context),
            None
        );

        let mut painter = RecordingPainter::default();
        checkbox.paint(&mut painter, SURFACE, &Theme::DARK).unwrap();
        assert_eq!(
            painter
                .ops
                .iter()
                .filter(|op| matches!(op, PaintOp::Fill(..)))
                .count(),
            2
        );
        assert!(matches!(painter.ops.last(), Some(PaintOp::Focus(..))));
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [BOUNDS]
        );
    }

    #[test]
    fn programmatic_checkbox_and_control_setters_are_idle_when_unchanged() {
        let mut checkbox = Checkbox::new(ID, BOUNDS, "Audio");
        let mut context = context();
        checkbox.set_checked(false, &mut context);
        checkbox.set_focused(false, &mut context);
        checkbox.set_enabled(true, &mut context);
        checkbox.set_label("Audio", &mut context);
        assert!(!context.has_damage());
        checkbox.set_checked(true, &mut context);
        assert_eq!(
            context.damaged_rects().collect::<std::vec::Vec<_>>(),
            [BOUNDS]
        );
    }

    #[test]
    fn controls_do_not_paint_without_a_clip_intersection() {
        let label = Label::new(ID, BOUNDS, "clip");
        let button = Button::new(ID, BOUNDS, "clip");
        let checkbox = Checkbox::new(ID, BOUNDS, "clip");

        let mut painter = RecordingPainter::default();
        label
            .paint(&mut painter, UiRect::EMPTY, &Theme::DARK)
            .unwrap();
        button
            .paint(&mut painter, UiRect::EMPTY, &Theme::DARK)
            .unwrap();
        checkbox
            .paint(&mut painter, UiRect::EMPTY, &Theme::DARK)
            .unwrap();
        assert!(painter.ops.is_empty());
    }

    #[test]
    fn narrow_checkbox_clips_box_and_omits_invalid_label_geometry() {
        let checkbox = Checkbox::new(ID, UiRect::new(0, 0, 4, 4), "too narrow");
        let mut painter = RecordingPainter::default();
        checkbox.paint(&mut painter, SURFACE, &Theme::DARK).unwrap();
        assert!(!painter.ops.iter().any(|op| matches!(op, PaintOp::Text(..))));
    }

    #[test]
    fn checkbox_mark_is_centered_and_accepts_a_bounded_offset() {
        let checkbox =
            Checkbox::new(ID, UiRect::new(10, 20, 80, 28), "offset").with_mark_offset(4, -2);
        let mut painter = RecordingPainter::default();
        checkbox.paint(&mut painter, SURFACE, &Theme::DARK).unwrap();

        assert_eq!(checkbox.mark_offset(), (4, -2));
        assert!(painter.ops.iter().any(|op| matches!(
            op,
            PaintOp::Stroke(
                _,
                UiRect {
                    x: 14,
                    y: 26,
                    w: 12,
                    h: 12
                },
                _,
                _
            )
        )));
    }

    #[test]
    fn control_sizes_match_documented_host_measurements() {
        assert_eq!(size_of::<Label<'static>>(), 48);
        assert_eq!(size_of::<Button<'static>>(), 64);
        assert_eq!(size_of::<Checkbox<'static>>(), 64);
    }

    #[test]
    fn released_phase_is_distinct_from_press_and_repeat() {
        assert_ne!(EventPhase::Released, EventPhase::Pressed);
        assert_ne!(EventPhase::Released, EventPhase::Repeated);
    }
}
