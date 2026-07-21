//! Native, allocation-free KotoConfig language surface.

use koto_ui::{
    Button, EventPhase, FocusEntry, FocusManager, FocusScopeId, Label, Navigation, PaintError,
    Painter, Panel, ResponseKind, TextAlign, TextRun, Theme, UiAction, UiContext, UiEvent, UiRect,
    WidgetId,
};

use crate::{
    ConfigCapability, ConfigPageId, ConfigPageRegistry, ConfigService, Locale, SntpServer,
    UtcOffset,
};

pub const KOTOCONFIG_SURFACE: UiRect = UiRect::new(0, 0, 320, 320);
const ENGLISH: WidgetId = WidgetId::new(1);
const JAPANESE: WidgetId = WidgetId::new(2);
const WIFI: WidgetId = WidgetId::new(5);
const OFFSET_MINUS: WidgetId = WidgetId::new(6);
const OFFSET_PLUS: WidgetId = WidgetId::new(7);
const SNTP_SERVER: WidgetId = WidgetId::new(10);
const PANEL_BOUNDS: UiRect = UiRect::new(8, 8, 304, 304);
const ENGLISH_BOUNDS: UiRect = UiRect::new(20, 76, 132, 28);
const JAPANESE_BOUNDS: UiRect = UiRect::new(168, 76, 132, 28);
const OFFSET_MINUS_BOUNDS: UiRect = UiRect::new(20, 174, 44, 28);
const OFFSET_VALUE_BOUNDS: UiRect = UiRect::new(80, 174, 160, 28);
const OFFSET_PLUS_BOUNDS: UiRect = UiRect::new(256, 174, 44, 28);
const SNTP_SERVER_BOUNDS: UiRect = UiRect::new(132, 244, 168, 28);
const WIFI_BOUNDS: UiRect = UiRect::new(20, 282, 280, 26);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KotoConfigAction {
    None,
    LocaleChanged(Locale),
    UtcOffsetChanged(UtcOffset),
    SntpServerChanged(SntpServer),
    OpenWifi,
    Exit,
}

pub struct KotoConfigUi {
    context: UiContext<12>,
    focus: FocusManager<6>,
    panel: Panel<'static>,
    heading: Label<'static>,
    current: Label<'static>,
    time_heading: Label<'static>,
    dst_notice: Label<'static>,
    server_heading: Label<'static>,
    english: Button<'static>,
    japanese: Button<'static>,
    wifi: Button<'static>,
    offset_minus: Button<'static>,
    offset_plus: Button<'static>,
    sntp_server: Button<'static>,
    utc_offset: UtcOffset,
    locale: Locale,
    pages: ConfigPageRegistry,
}

impl KotoConfigUi {
    pub fn new(config: &ConfigService) -> Self {
        Self::new_with_capabilities(config, ConfigCapability::LOCALE_CONFIG)
    }

    pub fn new_with_capabilities(config: &ConfigService, capabilities: ConfigCapability) -> Self {
        let locale = config.locale();
        let pages = ConfigPageRegistry::from_capabilities(
            capabilities.union(ConfigCapability::LOCALE_CONFIG),
        );
        let wifi_available = pages
            .pages()
            .iter()
            .any(|page| page.id == ConfigPageId::Wifi);
        let mut context = UiContext::new(KOTOCONFIG_SURFACE, Theme::DARK);
        let mut focus = FocusManager::new();
        focus
            .register(FocusEntry::new(ENGLISH, ENGLISH_BOUNDS, FocusScopeId::ROOT))
            .expect("KotoConfig English focus entry fits");
        focus
            .register(FocusEntry::new(
                JAPANESE,
                JAPANESE_BOUNDS,
                FocusScopeId::ROOT,
            ))
            .expect("KotoConfig Japanese focus entry fits");
        if wifi_available {
            focus
                .register(FocusEntry::new(WIFI, WIFI_BOUNDS, FocusScopeId::ROOT))
                .expect("KotoConfig Wi-Fi focus entry fits");
        }
        focus
            .register(FocusEntry::new(
                OFFSET_MINUS,
                OFFSET_MINUS_BOUNDS,
                FocusScopeId::ROOT,
            ))
            .expect("KotoConfig offset minus focus entry fits");
        focus
            .register(FocusEntry::new(
                OFFSET_PLUS,
                OFFSET_PLUS_BOUNDS,
                FocusScopeId::ROOT,
            ))
            .expect("KotoConfig offset plus focus entry fits");
        if wifi_available {
            focus
                .register(FocusEntry::new(
                    SNTP_SERVER,
                    SNTP_SERVER_BOUNDS,
                    FocusScopeId::ROOT,
                ))
                .expect("KotoConfig SNTP server focus entry fits");
        }
        focus
            .focus(
                if locale == Locale::JaJp {
                    JAPANESE
                } else {
                    ENGLISH
                },
                &mut context,
            )
            .expect("registered language choice accepts focus");

        let mut ui = Self {
            context,
            focus,
            panel: Panel::new(PANEL_BOUNDS).with_title(title(locale)),
            heading: Label::new(
                WidgetId::new(3),
                UiRect::new(20, 42, 280, 20),
                language_heading(locale),
            ),
            current: Label::new(
                WidgetId::new(4),
                UiRect::new(20, 122, 280, 20),
                current_language(locale),
            ),
            time_heading: Label::new(
                WidgetId::new(8),
                UiRect::new(20, 146, 280, 20),
                time_heading(locale),
            ),
            dst_notice: Label::new(
                WidgetId::new(9),
                UiRect::new(20, 212, 280, 20),
                dst_notice(locale),
            ),
            server_heading: Label::new(
                WidgetId::new(11),
                UiRect::new(20, 248, 108, 20),
                server_heading(locale),
            ),
            english: Button::new(ENGLISH, ENGLISH_BOUNDS, "English"),
            japanese: Button::new(JAPANESE, JAPANESE_BOUNDS, "日本語"),
            wifi: Button::new(WIFI, WIFI_BOUNDS, wifi_label(locale)),
            offset_minus: Button::new(OFFSET_MINUS, OFFSET_MINUS_BOUNDS, "-"),
            offset_plus: Button::new(OFFSET_PLUS, OFFSET_PLUS_BOUNDS, "+"),
            sntp_server: Button::new(
                SNTP_SERVER,
                SNTP_SERVER_BOUNDS,
                config.sntp_server().hostname(),
            ),
            utc_offset: config.utc_offset(),
            locale,
            pages,
        };
        ui.sync_focus();
        ui.context.clear_damage();
        ui
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    pub fn focused(&self) -> Option<WidgetId> {
        self.focus.focused()
    }

    pub fn wifi_available(&self) -> bool {
        self.pages
            .pages()
            .iter()
            .any(|page| page.id == ConfigPageId::Wifi)
    }

    pub fn damaged_rects(&self) -> impl Iterator<Item = UiRect> + '_ {
        self.context.damaged_rects()
    }

    pub fn clear_damage(&mut self) {
        self.context.clear_damage();
    }

    pub fn handle_event(&mut self, event: UiEvent, config: &mut ConfigService) -> KotoConfigAction {
        self.context.clear_damage();
        if event.phase == EventPhase::Pressed && event.action == UiAction::Cancel {
            return KotoConfigAction::Exit;
        }

        if event.phase != EventPhase::Released {
            if let UiAction::Navigate(direction) = event.action {
                let consumed = match direction {
                    Navigation::Next => {
                        let _ = self.focus.move_id_next(&mut self.context);
                        true
                    }
                    Navigation::Previous => {
                        let _ = self.focus.move_id_previous(&mut self.context);
                        true
                    }
                    Navigation::Up | Navigation::Down | Navigation::Left | Navigation::Right => {
                        matches!(
                            self.focus.move_spatial(direction, &mut self.context),
                            Ok(Some(_))
                        )
                    }
                    Navigation::PageUp | Navigation::PageDown => false,
                };
                if consumed {
                    self.sync_focus();
                    return KotoConfigAction::None;
                }
            }
        }

        let response = match self.focus.focused() {
            Some(ENGLISH) => self.english.handle_event(event, &mut self.context),
            Some(JAPANESE) => self.japanese.handle_event(event, &mut self.context),
            Some(WIFI) => self.wifi.handle_event(event, &mut self.context),
            Some(OFFSET_MINUS) => self.offset_minus.handle_event(event, &mut self.context),
            Some(OFFSET_PLUS) => self.offset_plus.handle_event(event, &mut self.context),
            Some(SNTP_SERVER) => self.sntp_server.handle_event(event, &mut self.context),
            _ => None,
        };
        let selected = if matches!(
            response.map(|item| item.kind),
            Some(ResponseKind::Activated)
        ) && event.phase == EventPhase::Pressed
        {
            match self.focus.focused() {
                Some(ENGLISH) => Some(Locale::EnUs),
                Some(JAPANESE) => Some(Locale::JaJp),
                Some(WIFI) => return KotoConfigAction::OpenWifi,
                Some(OFFSET_MINUS) => return self.change_offset(config, -UtcOffset::STEP_MINUTES),
                Some(OFFSET_PLUS) => return self.change_offset(config, UtcOffset::STEP_MINUTES),
                Some(SNTP_SERVER) => return self.change_sntp_server(config),
                _ => None,
            }
        } else {
            None
        };

        self.sync_focus();

        if let Some(locale) = selected {
            if config.set_locale(locale) {
                self.apply_locale(locale);
                return KotoConfigAction::LocaleChanged(locale);
            }
        }
        KotoConfigAction::None
    }

    pub fn paint(&self, painter: &mut impl Painter, clip: UiRect) -> Result<(), PaintError> {
        self.panel.paint(painter, clip, &Theme::DARK)?;
        self.heading.paint(painter, clip, &Theme::DARK)?;
        self.english.paint(painter, clip, &Theme::DARK)?;
        self.japanese.paint(painter, clip, &Theme::DARK)?;
        self.time_heading.paint(painter, clip, &Theme::DARK)?;
        self.offset_minus.paint(painter, clip, &Theme::DARK)?;
        self.offset_plus.paint(painter, clip, &Theme::DARK)?;
        let offset_text = OffsetText::new(self.utc_offset);
        painter.draw_text(
            clip,
            OFFSET_VALUE_BOUNDS,
            TextRun {
                text: offset_text.as_str(),
                color: Theme::DARK.normal.foreground,
                align: TextAlign::Center,
            },
        )?;
        self.dst_notice.paint(painter, clip, &Theme::DARK)?;
        if self.wifi_available() {
            self.server_heading.paint(painter, clip, &Theme::DARK)?;
            self.sntp_server.paint(painter, clip, &Theme::DARK)?;
            self.wifi.paint(painter, clip, &Theme::DARK)?;
        }
        self.current.paint(painter, clip, &Theme::DARK)
    }

    fn apply_locale(&mut self, locale: Locale) {
        self.locale = locale;
        self.panel = Panel::new(PANEL_BOUNDS).with_title(title(locale));
        self.heading
            .set_text(language_heading(locale), &mut self.context);
        self.current
            .set_text(current_language(locale), &mut self.context);
        self.time_heading
            .set_text(time_heading(locale), &mut self.context);
        self.dst_notice
            .set_text(dst_notice(locale), &mut self.context);
        self.server_heading
            .set_text(server_heading(locale), &mut self.context);
        self.wifi.set_label(wifi_label(locale), &mut self.context);
        self.context.damage(PANEL_BOUNDS);
    }

    fn sync_focus(&mut self) {
        let focused = self.focus.focused();
        self.english
            .set_focused(focused == Some(ENGLISH), &mut self.context);
        self.japanese
            .set_focused(focused == Some(JAPANESE), &mut self.context);
        self.wifi
            .set_focused(focused == Some(WIFI), &mut self.context);
        self.offset_minus
            .set_focused(focused == Some(OFFSET_MINUS), &mut self.context);
        self.offset_plus
            .set_focused(focused == Some(OFFSET_PLUS), &mut self.context);
        self.sntp_server
            .set_focused(focused == Some(SNTP_SERVER), &mut self.context);
    }

    fn change_offset(&mut self, config: &mut ConfigService, delta: i16) -> KotoConfigAction {
        let current = self.utc_offset.minutes();
        let mut next = current.saturating_add(delta);
        if next < UtcOffset::MINUTES_MIN {
            next = UtcOffset::MINUTES_MAX;
        } else if next > UtcOffset::MINUTES_MAX {
            next = UtcOffset::MINUTES_MIN;
        }
        let offset = UtcOffset::from_minutes(next).unwrap_or_default();
        if config.set_utc_offset(offset) {
            self.utc_offset = offset;
            self.context.damage(OFFSET_VALUE_BOUNDS);
            KotoConfigAction::UtcOffsetChanged(offset)
        } else {
            KotoConfigAction::None
        }
    }

    fn change_sntp_server(&mut self, config: &mut ConfigService) -> KotoConfigAction {
        let server = config.sntp_server().next();
        if config.set_sntp_server(server) {
            self.sntp_server
                .set_label(server.hostname(), &mut self.context);
            KotoConfigAction::SntpServerChanged(server)
        } else {
            KotoConfigAction::None
        }
    }
}

struct OffsetText {
    bytes: [u8; 9],
}

impl OffsetText {
    fn new(offset: UtcOffset) -> Self {
        let minutes = i32::from(offset.minutes());
        let absolute = minutes.abs();
        let hours = absolute / 60;
        let mins = absolute % 60;
        Self {
            bytes: [
                b'U',
                b'T',
                b'C',
                if minutes < 0 { b'-' } else { b'+' },
                b'0' + (hours / 10) as u8,
                b'0' + (hours % 10) as u8,
                b':',
                b'0' + (mins / 10) as u8,
                b'0' + (mins % 10) as u8,
            ],
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes).unwrap_or("UTC+00:00")
    }
}

fn wifi_label(locale: Locale) -> &'static str {
    match locale {
        Locale::EnUs => "Wi-Fi settings",
        Locale::JaJp => "Wi-Fi 設定",
        Locale::QpsPloc => "[!! Wi-Fi settings--- !!]",
    }
}

fn time_heading(locale: Locale) -> &'static str {
    match locale {
        Locale::EnUs => "Time zone (fixed UTC offset)",
        Locale::JaJp => "タイムゾーン（固定UTC差）",
        Locale::QpsPloc => "[!! Time zone (fixed UTC offset)--- !!]",
    }
}

fn dst_notice(locale: Locale) -> &'static str {
    match locale {
        Locale::EnUs => "Daylight-saving changes are not automatic",
        Locale::JaJp => "夏時間への切り替えは自動ではありません",
        Locale::QpsPloc => "[!! Daylight-saving is not automatic--- !!]",
    }
}

fn server_heading(locale: Locale) -> &'static str {
    match locale {
        Locale::EnUs => "NTP server",
        Locale::JaJp => "NTPサーバー",
        Locale::QpsPloc => "[!! NTP server--- !!]",
    }
}

fn title(locale: Locale) -> &'static str {
    match locale {
        Locale::EnUs => "KotoConfig",
        Locale::JaJp => "KotoConfig 設定",
        Locale::QpsPloc => "[!! KotoConfig Settings--- !!]",
    }
}

fn language_heading(locale: Locale) -> &'static str {
    match locale {
        Locale::EnUs => "Language",
        Locale::JaJp => "言語",
        Locale::QpsPloc => "[!! Language settings--- !!]",
    }
}

fn current_language(locale: Locale) -> &'static str {
    match locale {
        Locale::EnUs => "Current language: English",
        Locale::JaJp => "現在の言語: 日本語",
        Locale::QpsPloc => "[!! Current language: Pseudolocale--- !!]",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CONFIG_FORMAT_MAX_BYTES;
    use core::mem::size_of;
    use koto_ui::{GlyphRun, Rgb565, TextMetrics, TextRun};

    #[derive(Default)]
    struct CommandCounter(usize);

    impl TextMetrics for CommandCounter {
        fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
            Ok(text.len() as i32 * 6)
        }
    }

    impl Painter for CommandCounter {
        fn fill_rect(&mut self, _: UiRect, _: UiRect, _: Rgb565) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }

        fn stroke_rect(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }

        fn draw_text(&mut self, _: UiRect, _: UiRect, _: TextRun<'_>) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }

        fn draw_glyphs(&mut self, _: UiRect, _: UiRect, _: GlyphRun<'_>) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }

        fn draw_focus_mark(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            self.0 += 1;
            Ok(())
        }
    }

    #[test]
    fn current_locale_starts_focused_and_idle() {
        let mut config = ConfigService::default();
        config.set_locale(Locale::JaJp);
        let ui = KotoConfigUi::new(&config);
        assert_eq!(ui.locale(), Locale::JaJp);
        assert_eq!(ui.focused(), Some(JAPANESE));
        assert_eq!(ui.damaged_rects().count(), 0);
    }

    #[test]
    fn keyboard_selection_changes_shared_config_and_damages_panel() {
        let mut config = ConfigService::default();
        let mut ui = KotoConfigUi::new_with_capabilities(
            &config,
            ConfigCapability::LOCALE_CONFIG.union(ConfigCapability::WIFI_CONFIG),
        );
        assert_eq!(ui.focused(), Some(ENGLISH));
        assert_eq!(
            ui.handle_event(
                UiEvent::pressed(UiAction::Navigate(Navigation::Right)),
                &mut config
            ),
            KotoConfigAction::None
        );
        assert_eq!(ui.focused(), Some(JAPANESE));
        assert_eq!(
            ui.handle_event(UiEvent::pressed(UiAction::Activate), &mut config),
            KotoConfigAction::LocaleChanged(Locale::JaJp)
        );
        assert_eq!(config.locale(), Locale::JaJp);
        assert_eq!(config.locale_generation(), 2);
        assert!(ui.damaged_rects().any(|rect| rect == PANEL_BOUNDS));
    }

    #[test]
    fn selecting_existing_locale_is_idle_and_cancel_exits() {
        let mut config = ConfigService::default();
        let mut ui = KotoConfigUi::new(&config);
        assert_eq!(
            ui.handle_event(UiEvent::pressed(UiAction::Activate), &mut config),
            KotoConfigAction::None
        );
        assert_eq!(config.generation(), 1);
        assert_eq!(
            ui.handle_event(UiEvent::pressed(UiAction::Cancel), &mut config),
            KotoConfigAction::Exit
        );
    }

    #[test]
    fn pseudolocale_is_testable_but_choices_remain_self_identifying() {
        let mut config = ConfigService::default();
        config.set_locale(Locale::QpsPloc);
        let ui = KotoConfigUi::new(&config);
        assert_eq!(ui.english.label(), "English");
        assert_eq!(ui.japanese.label(), "日本語");
        assert!(ui.heading.text().contains("---"));
    }

    #[test]
    fn wifi_entry_exists_only_for_composite_capability() {
        let config = ConfigService::default();
        assert!(!KotoConfigUi::new(&config).wifi_available());

        let mut ui = KotoConfigUi::new_with_capabilities(
            &config,
            ConfigCapability::LOCALE_CONFIG.union(ConfigCapability::WIFI_CONFIG),
        );
        assert!(ui.wifi_available());
        let mut config = config;
        for _ in 0..2 {
            assert_eq!(
                ui.handle_event(
                    UiEvent::pressed(UiAction::Navigate(Navigation::Next)),
                    &mut config,
                ),
                KotoConfigAction::None
            );
        }
        assert_eq!(ui.focused(), Some(WIFI));
        assert_eq!(
            ui.handle_event(UiEvent::pressed(UiAction::Activate), &mut config),
            KotoConfigAction::OpenWifi
        );
    }

    #[test]
    fn resident_write_and_damage_budgets_remain_bounded() {
        let mut config = ConfigService::default();
        let mut ui = KotoConfigUi::new(&config);
        let config_bytes = size_of::<ConfigService>();
        let ui_bytes = size_of::<KotoConfigUi>();

        println!(
            "KotoConfig budgets: service={config_bytes} ui={ui_bytes} write_max={CONFIG_FORMAT_MAX_BYTES}"
        );
        assert!(config_bytes <= 256);
        assert!(ui_bytes <= 1152);
        assert_eq!(CONFIG_FORMAT_MAX_BYTES, 280);
        assert_eq!(ui.damaged_rects().count(), 0);
        let mut commands = CommandCounter::default();
        ui.paint(&mut commands, KOTOCONFIG_SURFACE).unwrap();
        println!("KotoConfig render commands={}", commands.0);
        assert!(commands.0 <= 30);

        ui.handle_event(
            UiEvent::pressed(UiAction::Navigate(Navigation::Right)),
            &mut config,
        );
        ui.clear_damage();
        assert_eq!(
            ui.handle_event(UiEvent::pressed(UiAction::Activate), &mut config),
            KotoConfigAction::LocaleChanged(Locale::JaJp)
        );
        let mut damage = ui.damaged_rects();
        assert_eq!(damage.next(), Some(PANEL_BOUNDS));
        assert_eq!(damage.next(), None);
    }

    #[test]
    fn offset_controls_use_fifteen_minutes_and_show_dst_warning() {
        let mut config = ConfigService::default();
        let mut ui = KotoConfigUi::new(&config);
        ui.focus
            .focus(OFFSET_PLUS, &mut ui.context)
            .expect("offset control is registered");
        ui.sync_focus();
        assert_eq!(
            ui.handle_event(UiEvent::pressed(UiAction::Activate), &mut config),
            KotoConfigAction::UtcOffsetChanged(UtcOffset::from_minutes(15).unwrap())
        );
        assert_eq!(config.utc_offset().minutes(), 15);
        assert!(dst_notice(Locale::EnUs).contains("not automatic"));
        assert!(dst_notice(Locale::JaJp).contains("自動ではありません"));
        assert!(time_heading(Locale::EnUs).starts_with("Time zone"));
        assert!(time_heading(Locale::JaJp).starts_with("タイムゾーン"));
        assert_eq!(OffsetText::new(config.utc_offset()).as_str(), "UTC+00:15");
    }

    #[test]
    fn sntp_server_button_cycles_curated_endpoints() {
        let mut config = ConfigService::default();
        let mut ui = KotoConfigUi::new_with_capabilities(
            &config,
            ConfigCapability::LOCALE_CONFIG.union(ConfigCapability::WIFI_CONFIG),
        );
        ui.focus
            .focus(SNTP_SERVER, &mut ui.context)
            .expect("SNTP server control is registered");
        ui.sync_focus();
        assert_eq!(
            ui.handle_event(UiEvent::pressed(UiAction::Activate), &mut config),
            KotoConfigAction::SntpServerChanged(SntpServer::NictJapan)
        );
        assert_eq!(config.sntp_server(), SntpServer::NictJapan);
        assert_eq!(ui.sntp_server.label(), "ntp.nict.jp");
    }
}
