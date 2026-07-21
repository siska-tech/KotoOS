use koto_core::BitmapFont;
use koto_sim::{GalleryResponse, UiGallery};
use koto_ui::{
    DamageSet, Dialog, DialogAction, FocusEntry, FocusManager, FocusScopeId, List, ListModel,
    ListRow, Navigation, Panel, RegistrationError, ResponseKind, Theme, UiAction, UiContext,
    UiEvent, UiRect, Utf8Buffer, WidgetId,
};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/mplus12.kfont");
const DEFAULT_GOLDEN: u64 = 0x918a8f593f2babb7;
const MODAL_GOLDEN: u64 = 0x5ee394e7789fc0be;

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn pixels(mut gallery: UiGallery, font: &BitmapFont<'_>) -> Vec<u8> {
    gallery.render(font).as_canvas().pixels().to_vec()
}

#[test]
fn default_and_modal_frames_match_320_square_goldens() {
    let font = BitmapFont::from_bytes(FONT).unwrap();
    let default = hash(&pixels(UiGallery::new(), &font));
    let mut modal = UiGallery::new();
    modal.handle_event(UiEvent::pressed(UiAction::Activate));
    let modal = hash(&pixels(modal, &font));
    println!("gallery default={default:#018x} modal={modal:#018x}");
    assert_eq!(default, DEFAULT_GOLDEN);
    assert_eq!(modal, MODAL_GOLDEN);
}

#[test]
fn deterministic_scenario_traces_focus_responses_and_damage() {
    let mut gallery = UiGallery::new();
    let next = UiEvent::pressed(UiAction::Navigate(Navigation::Next));
    let step = gallery.handle_event(next);
    assert_eq!(step.focused, Some(WidgetId::new(102)));
    assert_eq!(
        step.damage,
        [UiRect::new(20, 66, 120, 24), UiRect::new(164, 66, 136, 24)]
    );
    let step = gallery.handle_event(UiEvent::pressed(UiAction::Activate));
    assert!(
        matches!(step.response, Some(GalleryResponse::Control(response)) if response.kind == ResponseKind::ValueChanged(1))
    );
    assert_eq!(step.damage, [UiRect::new(164, 66, 136, 24)]);
    assert!(gallery.checkbox_is_checked());

    assert_eq!(gallery.handle_event(next).focused, Some(WidgetId::new(103)));
    let selected = gallery.handle_event(UiEvent::pressed(UiAction::Navigate(Navigation::Down)));
    assert!(
        matches!(selected.response, Some(GalleryResponse::Control(response)) if response.kind == ResponseKind::SelectionChanged(1))
    );
    for _ in 0..8 {
        gallery.handle_event(UiEvent::repeated(UiAction::Navigate(Navigation::Down)));
    }
    assert!(gallery.list_first_visible() > 0);
    assert_eq!(gallery.handle_event(next).focused, Some(WidgetId::new(104)));
    assert!(!gallery.text_is_editing());
    let activate_field = gallery.handle_event(UiEvent::pressed(UiAction::Activate));
    assert!(gallery.text_is_editing());
    assert_eq!(activate_field.damage, [UiRect::new(176, 104, 124, 24)]);

    let edit = gallery.handle_event(UiEvent::pressed(UiAction::Text('あ')));
    assert!(
        matches!(edit.response, Some(GalleryResponse::Control(response)) if response.kind == ResponseKind::TextChanged(3))
    );
    assert_eq!(edit.damage, [UiRect::new(176, 104, 124, 24)]);
    assert_eq!(gallery.text(), "あ");
    assert_eq!(
        gallery.set_composition(true).damage,
        [UiRect::new(176, 104, 124, 24)]
    );
    assert!(gallery.composition_is_visible());

    let previous = UiEvent::pressed(UiAction::Navigate(Navigation::Previous));
    gallery.handle_event(previous);
    gallery.handle_event(previous);
    gallery.handle_event(previous);
    let modal = gallery.handle_event(UiEvent::pressed(UiAction::Activate));
    assert!(gallery.dialog_is_open());
    assert_eq!(modal.focused, Some(WidgetId::new(111)));
    assert_eq!(modal.damage, [UiRect::new(0, 0, 320, 320)]);
    let cancel = gallery.handle_event(UiEvent::pressed(UiAction::Cancel));
    assert!(
        matches!(cancel.response, Some(GalleryResponse::Dialog(result)) if result.dialog == WidgetId::new(110))
    );
    assert_eq!(cancel.damage, [UiRect::new(0, 0, 320, 320)]);
}

#[test]
fn repeated_idle_frame_emits_no_damage_or_paint_work() {
    let font = BitmapFont::from_bytes(FONT).unwrap();
    let mut gallery = UiGallery::new();
    let mut framebuffer = gallery.render(&font);
    gallery.clear_damage();
    assert_eq!(gallery.paint_damage(&font, &mut framebuffer), 0);
    assert_eq!(gallery.paint_damage(&font, &mut framebuffer), 0);
}

struct HugeModel;
impl ListModel for HugeModel {
    fn len(&self) -> usize {
        usize::MAX
    }
    fn row(&self, _: usize) -> Option<ListRow<'_>> {
        Some(ListRow::new("row"))
    }
}

#[test]
fn bounded_capacity_paths_fail_without_panics_or_growth() {
    let mut focus = FocusManager::<0>::new();
    assert_eq!(
        focus.register(FocusEntry::new(
            WidgetId::new(1),
            UiRect::new(0, 0, 1, 1),
            FocusScopeId::ROOT
        )),
        Err(RegistrationError::Capacity)
    );
    let mut damage = DamageSet::<0>::new(UiRect::new(0, 0, 10, 10));
    damage.push(UiRect::new(1, 1, 1, 1));
    assert_eq!(
        damage.iter().collect::<Vec<_>>(),
        [UiRect::new(0, 0, 10, 10)]
    );
    let mut bytes = [0; 1];
    let mut text = Utf8Buffer::from_str(&mut bytes, "x").unwrap();
    assert!(text.insert_str(1, "y").is_err());
    let mut dialog: Dialog<1, 1> = Dialog::new(
        WidgetId::new(2),
        FocusScopeId::new(2),
        Panel::new(UiRect::new(0, 0, 20, 20)),
    );
    dialog
        .add_action(DialogAction::new(WidgetId::new(3)))
        .unwrap();
    assert!(dialog
        .add_action(DialogAction::new(WidgetId::new(4)))
        .is_err());
    let mut list = List::new(WidgetId::new(5), UiRect::new(0, 0, 20, 20), 10);
    let mut context = UiContext::<2>::new(UiRect::new(0, 0, 20, 20), Theme::DARK);
    list.sync_model(&HugeModel, &mut context);
    assert!(list.selected().is_some());
}
