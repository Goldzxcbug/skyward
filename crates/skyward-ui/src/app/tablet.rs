//! The navigation column and detail viewport of the landscape layout.

use super::*;
use crate::{color, icons};

const GROUPS: &[&[pages::HomeEntry]] = &[
    &pages::HOME_DEVICE,
    &pages::HOME_CONNECTIONS,
    &pages::HOME_TOOLS,
    &pages::HOME_SYSTEM,
];

pub(super) struct Tablet {
    pub width: u32,
    pub detail: Option<Canvas>,
    pub charge_dither: Option<glow::Dither>,
    scroll: f32,
    sidebar: Option<(SidebarView, Canvas)>,
}

impl Tablet {
    pub(super) fn new(w: u32, h: u32, density: f32) -> Option<Self> {
        if !density.is_finite()
            || density <= 0.0
            || w <= h
            || w as f32 / density < 840.0
            || h as f32 / density < 540.0
        {
            return None;
        }
        let width = (w as f32 * 0.32)
            .clamp(240.0 * density, 430.0 * density)
            .round() as u32;
        Some(Self {
            width,
            detail: None,
            charge_dither: None,
            scroll: 0.0,
            sidebar: None,
        })
    }
}

struct Menu {
    search: Rect,
    top: f32,
    bottom: f32,
    end: f32,
    cards: Vec<Rect>,
    entries: Vec<(pages::HomeEntry, Rect)>,
}

impl Menu {
    fn new(t: &Theme, state: &State) -> Self {
        let m = &t.m;
        let width = m.w as f32 - 2.0 * m.margin;
        let top = m.dp(170.0).round();
        let bottom = m.h as f32 - m.dp(43.0).round();
        let mut menu = Self {
            search: Rect::new(m.margin, m.dp(110.0), width, m.search_h),
            top,
            bottom,
            end: top,
            cards: Vec::new(),
            entries: Vec::new(),
        };
        let mut y = top;
        for group in GROUPS {
            let first = y;
            for entry in group.iter().filter(|entry| state.offers(entry.screen)) {
                let height = m.dp(65.0).round();
                menu.entries
                    .push((*entry, Rect::new(m.margin, y, width, height)));
                y += height;
            }
            if y > first {
                menu.cards
                    .push(Rect::new(m.margin, first, width, y - first));
                menu.end = y;
                y += m.card_gap;
            }
        }
        menu
    }

    fn max_scroll(&self) -> f32 {
        (self.end - self.bottom).max(0.0)
    }

    fn hit(&self, x: f32, y: f32, scroll: f32) -> Option<Screen> {
        if self.search.contains(x, y) {
            return Some(Screen::Search);
        }
        if y < self.top || y >= self.bottom {
            return None;
        }
        self.entries
            .iter()
            .find(|(_, rect)| rect.contains(x, y + scroll))
            .map(|(entry, _)| entry.screen)
    }
}

#[derive(PartialEq)]
struct SidebarView {
    dark: bool,
    lang: Lang,
    model: String,
    wlan: bool,
    systems: bool,
    active: bool,
    selected: Screen,
    pressed: Option<Screen>,
    focus: Option<(Rect, f32, Rgba)>,
    scroll: f32,
}

impl App {
    pub(super) fn home_screen(&self) -> Screen {
        if self.tablet.is_some() {
            Screen::About
        } else {
            Screen::Home
        }
    }

    pub(super) fn panel_theme(&self) -> Theme {
        let mut theme = self.theme();
        if let Some(tablet) = &self.tablet {
            theme.m.w += tablet.width;
            theme.m.status_x = theme.m.dp(27.0);
        }
        theme
    }

    fn sidebar_theme(&self) -> Theme {
        let mut theme = self.theme();
        theme.m.w = self.tablet.as_ref().expect("landscape layout").width;
        theme
    }

    /// A system request can own the recovery until it is dismissed.
    fn sidebar_active(&self) -> bool {
        self.tablet.is_some() && self.stack[0].0 == Screen::About
    }

    pub(super) fn compose_tablet(
        &mut self,
        out: &mut Canvas,
        f: &Fonts,
        info: &DeviceInfo,
        clock: &str,
        now: u64,
    ) {
        let tablet = self.tablet.as_mut().expect("landscape layout");
        let mut detail = tablet
            .detail
            .take()
            .unwrap_or_else(|| Canvas::new(self.w, self.h));
        self.compose_page(&mut detail, f, info, clock, now);
        let t = self.sidebar_theme();
        let active = self.sidebar_active();
        let selected = self.stack.get(1).map_or(Screen::About, |page| page.0);
        let pressed = match self.drag {
            Drag::Sidebar { pressed, .. } => pressed,
            _ => None,
        };
        let focus = self
            .light(self.top().1)
            .and_then(|(group, rect, radius, tint)| {
                (group == Group::Sidebar).then_some((rect, radius, tint))
            });
        let tablet = self.tablet.as_mut().expect("landscape layout");
        let menu = Menu::new(&t, &self.state);
        tablet.scroll = tablet.scroll.min(menu.max_scroll());
        let view = SidebarView {
            dark: t.dark,
            lang: self.state.lang,
            model: info.model.clone(),
            wlan: self.state.offers(Screen::Wlan),
            systems: self.state.offers(Screen::Systems),
            active,
            selected,
            pressed,
            focus,
            scroll: tablet.scroll.round(),
        };
        if tablet
            .sidebar
            .as_ref()
            .is_none_or(|(last, _)| last != &view)
        {
            let mut art = tablet
                .sidebar
                .take()
                .map_or_else(|| Canvas::new(tablet.width, self.h), |(_, art)| art);
            paint_sidebar(&mut art, &t, f, &menu, &view);
            tablet.sidebar = Some((view, art));
        }
        let (_, sidebar) = tablet.sidebar.as_ref().expect("sidebar drawn");
        out.put_pane(sidebar, 0);
        out.put_pane(&detail, tablet.width);
        tablet.detail = Some(detail);
        let t = self.panel_theme();
        widgets::status_bar(out, &t, f, &self.bar(clock, info), t.pal.status);
    }

    pub(super) fn sidebar_spots(&self) -> Vec<Spot> {
        if !self.sidebar_active() {
            return Vec::new();
        }
        let menu = Menu::new(&self.sidebar_theme(), &self.state);
        let spot = |screen, rect| Spot {
            stop: Stop::Hit(Action::Sidebar(screen)),
            group: Group::Sidebar,
            rect,
        };
        std::iter::once(spot(Screen::Search, menu.search))
            .chain(
                menu.entries
                    .into_iter()
                    .map(|(entry, rect)| spot(entry.screen, rect)),
            )
            .collect()
    }

    pub(super) fn sidebar_position(&self, s: &Spot) -> Rect {
        let y = if s.stop == Stop::Hit(Action::Sidebar(Screen::Search)) {
            s.rect.y
        } else {
            s.rect.y
                - self
                    .tablet
                    .as_ref()
                    .expect("landscape layout")
                    .scroll
                    .round()
        };
        Rect { y, ..s.rect }
    }

    pub(super) fn reveal_sidebar(&mut self, s: &Spot) {
        if s.stop == Stop::Hit(Action::Sidebar(Screen::Search)) {
            return;
        }
        let menu = Menu::new(&self.sidebar_theme(), &self.state);
        let tablet = self.tablet.as_mut().expect("landscape layout");
        tablet.scroll = tablet
            .scroll
            .clamp(
                (s.rect.y + s.rect.h - menu.bottom).max(0.0),
                s.rect.y - menu.top,
            )
            .min(menu.max_scroll());
    }

    pub(super) fn touch_sidebar(&mut self, event: Touch, now: u64) -> Vec<Effect> {
        let t = self.sidebar_theme();
        let menu = Menu::new(&t, &self.state);
        let active = self.sidebar_active();
        let tablet = self.tablet.as_mut().expect("landscape layout");
        tablet.scroll = tablet.scroll.min(menu.max_scroll());
        match event {
            Touch::Down(x, y) => {
                let pressed = active
                    .then(|| menu.hit(x, y, tablet.scroll.round()))
                    .flatten();
                self.drag = Drag::Sidebar {
                    start: (x, y),
                    last: y,
                    pressed,
                };
            }
            Touch::Move(x, y) => {
                if let Drag::Sidebar {
                    start,
                    last,
                    pressed,
                } = &mut self.drag
                {
                    let slop = t.m.dp(8.0);
                    if (x - start.0).abs() > slop
                        || (y - start.1).abs() > slop
                        || *pressed != menu.hit(x, y, tablet.scroll.round())
                    {
                        *pressed = None;
                    }
                    let scrolling = *last != start.1 || (y - start.1).abs() > slop;
                    if active && start.1 >= menu.top && start.1 < menu.bottom && scrolling {
                        tablet.scroll = (tablet.scroll + *last - y).clamp(0.0, menu.max_scroll());
                        *last = y;
                    }
                }
            }
            Touch::Up => {
                self.dirty = true;
                if let Drag::Sidebar {
                    pressed: Some(screen),
                    ..
                } = std::mem::replace(&mut self.drag, Drag::Idle)
                {
                    return self.select_section(screen, now);
                }
            }
        }
        self.dirty = true;
        Vec::new()
    }

    pub(super) fn select_section(&mut self, screen: Screen, now: u64) -> Vec<Effect> {
        let mut effects = Vec::new();
        if !self.sidebar_active()
            || !self.state.offers(screen)
            || (self.screen() == screen && self.stack.len() <= 2)
        {
            return effects;
        }
        // Go through the existing back handlers so a running job or an
        // unsaved editor can keep its page and explain why it did.
        while self.stack.len() > 1 {
            let depth = self.stack.len();
            effects.extend(self.back_page(now, false));
            if self.stack.len() == depth {
                return effects;
            }
        }
        if self.screen() != screen {
            self.open(screen);
            effects.extend(self.arrive_at(screen));
        }
        effects
    }
}

fn paint_sidebar(out: &mut Canvas, t: &Theme, f: &Fonts, menu: &Menu, view: &SidebarView) {
    let m = &t.m;
    let w = m.w as f32;
    let bg = if t.dark {
        Rgba::rgb(17, 18, 21)
    } else {
        Rgba::rgb(242, 244, 248)
    };
    out.clear(bg);
    if view.active {
        for card in &menu.cards {
            out.fill_round_rect(
                card.x,
                card.y - view.scroll,
                card.w,
                card.h,
                m.card_radius,
                t.pal.card,
            );
        }
        for (entry, rect) in &menu.entries {
            let y = rect.y - view.scroll;
            if y + rect.h <= menu.top || y >= menu.bottom {
                continue;
            }
            let selected = entry.screen == view.selected;
            let mut fill = if selected { t.pal.badge } else { t.pal.card };
            if view.pressed == Some(entry.screen) {
                fill = color::over(t.pal.row_press, fill);
            }
            if fill != t.pal.card {
                let inset = m.dp(4.0);
                out.fill_round_rect(
                    rect.x + inset,
                    y + inset,
                    rect.w - inset * 2.0,
                    rect.h - inset * 2.0,
                    m.dp(15.0),
                    fill,
                );
            }
            let cy = y + rect.h / 2.0;
            let icon = rect.x + m.inner;
            icons::tile(out, t, icon, cy - m.icon / 2.0, entry.tile.0, entry.tile.1);
            let ink = if selected { t.pal.accent } else { t.pal.text };
            let text_x = icon + m.icon + m.dp(14.0);
            let right = rect.x + rect.w - m.chevron_inset;
            out.text_vc(
                &f.heading,
                view.lang.pick(entry.title),
                text_x,
                cy,
                m.dp(18.0),
                ink,
                Some(right - text_x - m.dp(16.0)),
            );
            widgets::chevron(
                out,
                t,
                right,
                cy,
                if selected { ink } else { t.pal.chevron },
            );
            if let Some(card) = menu
                .cards
                .iter()
                .find(|card| card.y <= rect.y && rect.y < card.y + card.h)
                && rect.y + rect.h < card.y + card.h
                && !selected
            {
                let separator = color::over(t.pal.row_press, t.pal.card);
                out.fill_rect(
                    text_x,
                    (y + rect.h).round() - 1.0,
                    rect.x + rect.w - m.inner - text_x,
                    1.0,
                    separator,
                );
            }
        }
        if let Some((rect, radius, tint)) = view
            .focus
            .filter(|(r, _, _)| r.y >= menu.search.y + menu.search.h)
        {
            out.fill_round_rect(rect.x, rect.y, rect.w, rect.h, radius, tint);
        }
        if menu.max_scroll() > 0.0 {
            let visible = menu.bottom - menu.top;
            let h = visible * visible / (menu.end - menu.top);
            let y = menu.top + (visible - h) * view.scroll / menu.max_scroll();
            let stroke = m.dp(2.0);
            out.fill_round_rect(
                w - m.dp(5.0),
                y,
                stroke,
                h,
                stroke / 2.0,
                t.pal.chevron.with_alpha(130),
            );
        }
    }
    // These two bands stay put and cover the list where it scrolls past.
    out.fill_rect(0.0, 0.0, w, menu.top, bg);
    out.fill_rect(0.0, menu.bottom, w, m.h as f32 - menu.bottom, bg);
    let inset = m.dp(27.0);
    out.text_top(
        &f.heading,
        pages::HOME_TITLE,
        inset,
        m.dp(71.0),
        m.dp(27.0),
        t.pal.text,
        None,
    );
    if view.active {
        widgets::search_capsule(out, t, f, menu.search, view.lang.pick(pages::SEARCH_HINT));
        if view.pressed == Some(Screen::Search) {
            widgets::search_bar_held(out, t, menu.search);
        }
        if let Some((rect, radius, tint)) = view
            .focus
            .filter(|(r, _, _)| r.y < menu.search.y + menu.search.h)
        {
            out.fill_round_rect(rect.x, rect.y, rect.w, rect.h, radius, tint);
        }
    }
    out.text_vc(
        &f.value,
        &view.model,
        inset,
        menu.bottom + m.dp(19.0),
        m.dp(13.0),
        t.pal.subtitle,
        Some(w - inset * 2.0),
    );
    let line = m.dp(0.5).max(1.0).round();
    out.fill_rect(
        w - line,
        0.0,
        line,
        m.h as f32,
        color::over(t.pal.row_press, bg),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{battery_at, fonts, info, spot};
    use skyward_info::Charge;

    const W: u32 = 1280;
    const H: u32 = 906;

    fn tap_at(app: &mut App, fonts: &Fonts, x: f32, y: f32) -> Vec<Effect> {
        assert!(app.touch(Touch::Down(x, y), fonts, 0).is_empty());
        app.touch(Touch::Up, fonts, 40)
    }

    fn sidebar_tap(app: &mut App, fonts: &Fonts, screen: Screen) -> Vec<Effect> {
        let spot = app
            .sidebar_spots()
            .into_iter()
            .find(|s| s.stop == Stop::Hit(Action::Sidebar(screen)))
            .unwrap();
        app.reveal_sidebar(&spot);
        let (x, y) = app.sidebar_position(&spot).centre();
        tap_at(app, fonts, x, y)
    }

    #[test]
    fn landscape_starts_on_the_device_and_routes_each_pane_in_its_own_coordinates() {
        let (fonts, info) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, 1.0);
        let divider = app.tablet.as_ref().unwrap().width as f32;
        assert_eq!(app.screen(), Screen::About);
        assert!(!app.frame(&fonts, &info).back);
        assert!(
            !app.sidebar_spots()
                .iter()
                .any(|s| s.stop == Stop::Hit(Action::Sidebar(Screen::Wlan)))
        );
        let mut result = Canvas::new(W, H);
        app.compose(&mut result, &fonts, &info, "09:41", 0);
        let row = &result.data[..W as usize * 4];
        assert_ne!(
            &row[..4],
            &row[divider as usize * 4..divider as usize * 4 + 4]
        );
        let (x, y) = spot(&mut app, &fonts, &info, Action::Open(Screen::Specs));
        tap_at(&mut app, &fonts, divider + x, y);
        assert_eq!(app.screen(), Screen::Specs);
        assert_eq!(
            sidebar_tap(&mut app, &fonts, Screen::Mount),
            vec![Effect::ReadMounts]
        );
        assert_eq!(app.screen(), Screen::Mount);
        app.frame(&fonts, &info);
        let (x, y) = widgets::back_rect(&app.theme()).centre();
        tap_at(&mut app, &fonts, divider + x, y);
        assert_eq!(app.screen(), Screen::About);
        sidebar_tap(&mut app, &fonts, Screen::Search);
        assert_eq!(app.screen(), Screen::Search);
        assert!(app.morph.is_none());
    }

    #[test]
    fn sidebar_scroll_exposes_lower_entries_without_scrolling_or_activating_the_page() {
        let fonts = fonts();
        let mut app = App::new(Screen::Home, W, 600, 1.0);
        let menu = Menu::new(&app.sidebar_theme(), &app.state);
        app.touch(Touch::Down(65.0, 460.0), &fonts, 0);
        app.touch(Touch::Move(65.0, menu.top + 5.0), &fonts, 100);
        assert!(app.touch(Touch::Up, &fonts, 120).is_empty());
        assert!(app.tablet.as_ref().unwrap().scroll > 0.0);
        assert_eq!((app.screen(), app.top().1), (Screen::About, 0.0));
        assert_eq!(
            sidebar_tap(&mut app, &fonts, Screen::Reboot),
            vec![Effect::ReadSlots]
        );
    }

    #[test]
    fn sidebar_respects_a_write_in_progress_and_emergency_pages() {
        let fonts = fonts();
        let mut app = App::new(Screen::Home, W, H, 1.0);
        app.open(Screen::Install);
        app.open(Screen::Installing);
        app.state.install = Install::Flashing {
            partition: "boot",
            stage: None,
        };
        assert!(sidebar_tap(&mut app, &fonts, Screen::About).is_empty());
        assert_eq!(app.screen(), Screen::Installing);
        assert!(app.toast.is_some());

        let mut fastboot = App::new(Screen::Fastboot, W, H, 1.0);
        assert!(fastboot.sidebar_spots().is_empty());
        fastboot.enter_recovery();
        assert_eq!(fastboot.screen(), Screen::About);
        assert!(!fastboot.sidebar_spots().is_empty());
    }

    #[test]
    fn confirmation_track_uses_detail_coordinates_and_stays_captured() {
        let (fonts, info) = (fonts(), info());
        let mut app = App::new(Screen::Home, W, H, 1.0);
        let divider = app.tablet.as_ref().unwrap().width as f32;
        sidebar_tap(&mut app, &fonts, Screen::Reboot);
        let frame = app.frame(&fonts, &info);
        let track = frame
            .hits
            .iter()
            .find(|hit| hit.action == Action::Confirm(Confirm::Reboot))
            .unwrap()
            .rect;
        let y = track.centre().1;
        let knob = Touch::Down(divider + track.x + track.h / 2.0, y);
        app.touch(knob, &fonts, 100);
        app.touch(Touch::Move(divider - 1.0, y), &fonts, 140);
        assert!(app.touch(Touch::Up, &fonts, 160).is_empty());
        assert_eq!(app.screen(), Screen::Reboot);

        app.touch(knob, &fonts, 200);
        app.touch(Touch::Move(divider + track.x + track.w, y), &fonts, 240);
        assert_eq!(
            app.touch(Touch::Up, &fonts, 260),
            vec![Effect::Reboot(Target::System)]
        );
    }

    #[test]
    fn keys_reach_sidebar_entries_and_the_charge_overlay_uses_the_full_panel() {
        let fonts = fonts();
        let info = DeviceInfo {
            battery: Some(battery_at(70, Charge::Charging)),
            ..info()
        };
        let mut app = App::new(Screen::Home, W, H, 1.0);
        let mut result = Canvas::new(W, H);
        app.compose(&mut result, &fonts, &info, "09:41", 0);
        app.touchscreen_missing();
        assert_eq!(
            app.lit().unwrap().stop,
            Stop::Hit(Action::Sidebar(Screen::Search))
        );
        app.volume(true, 0);
        app.volume(true, 50);
        assert_eq!(
            app.lit().unwrap().stop,
            Stop::Hit(Action::Sidebar(Screen::Install))
        );
        app.plugged(100, true);
        app.compose(&mut result, &fonts, &info, "09:41", 450);
        app.compose(&mut result, &fonts, &info, "09:41", 580);
    }

    #[test]
    fn phone_and_narrow_landscape_panels_keep_the_single_page_layout() {
        for (w, h) in [(412, 920), (920, 412), (800, 600)] {
            let app = App::new(Screen::Home, w, h, 1.0);
            assert!(app.tablet.is_none());
            assert_eq!(app.screen(), Screen::Home);
            assert_eq!(app.theme().m.w, w);
        }
    }
}
