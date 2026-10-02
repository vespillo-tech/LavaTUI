//! The settings screen (`,`): every everyday preference in plain words,
//! page by page, and the guided Spotify setup (lava-1xk.17).
//!
//! Nothing here needs saving by hand: each change applies at once (the
//! lamp, clock and widgets show it live) and goes through the usual
//! debounced save. The screen is a list of pages (`look`, `clock & timer`,
//! …) and each page a list of [`Row`]s built fresh from the model, so it
//! always shows what is really set. Words on screen are for people who
//! have never seen the config file: no key names, no jargon.
//!
//! Geometry and drawing are in `ui/settings.rs`; this file is the state
//! and what keys and clicks do.

use std::time::{Duration, Instant};

use super::{Model, Overlay, depth_for, palette_named, pomodoro_config};
use crate::clock;
use crate::config::{
    CellsChoice, ColorChoice, LoginStore, MinimalClock, Overridden, Settings, UiMode,
};
use crate::disk_cache::size_words;
use crate::dock::cover::{CoverSize, Detail};
use crate::dock::{self, Anchor, Backing, Place, TextInk, WIDGETS};
use crate::media::{Status, Unavailable};
use crate::render::StyleId;
use crate::sim::{HEAT_LEVELS, SimSpeed};
use crate::spotify_web::REDIRECT_URI;
use crate::theme::Theme;
use crate::ui::keymap::Action;
use crate::ui::settings::{self as geometry, Hit};

use super::library::Account;
use super::pickers::PickerKind;

/// Where a Spotify developer app is made (setup step 1).
pub const DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";
/// A second enter within this resets a page or logs out.
const CONFIRM_WINDOW: Duration = Duration::from_secs(3);
/// The longest text the Client ID field takes (a real one is 32).
const FIELD_MAX: usize = 64;

/// Focus and break lengths offered, in minutes.
const MINUTES: &[u32] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 75, 90, 105, 120,
    150, 180,
];
/// Frame rates offered.
const FPS: &[u32] = &[10, 15, 20, 24, 30, 45, 60, 90, 120, 144, 165, 240];

/// A page of the settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Look,
    Clock,
    Widgets,
    Music,
    Controls,
    Window,
    /// The guided Spotify setup, reached from `music & lyrics`.
    Spotify,
}

impl Page {
    /// The pages in the list, in order (the Spotify setup isn't listed).
    pub const LIST: [Page; 6] = [
        Page::Look,
        Page::Clock,
        Page::Widgets,
        Page::Music,
        Page::Controls,
        Page::Window,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Page::Look => "look",
            Page::Clock => "clock & timer",
            Page::Widgets => "widgets",
            Page::Music => "music & lyrics",
            Page::Controls => "controls",
            Page::Window => "window",
            Page::Spotify => "spotify setup",
        }
    }

    /// What the page is for, shown while it's picked in the list.
    pub fn about(self) -> &'static str {
        match self {
            Page::Look => "How the lamp looks: its style, colours, heat and speed.",
            Page::Clock => "The clock face, and how long focus sessions and breaks last.",
            Page::Widgets => {
                "Where the clock, the focus timer and the music go: beside the lamp, \
                 on it, or nowhere."
            }
            Page::Music => {
                "Connecting Spotify, song lyrics and album covers, and clearing the saved ones."
            }
            Page::Controls => "How the mouse works in the lamp.",
            Page::Window => "Lamp-only mode, the hint line and how smoothly the lamp moves.",
            Page::Spotify => SPOTIFY_INTRO,
        }
    }

    /// Its place in [`Page::LIST`] (the setup sits under music).
    pub fn index(self) -> usize {
        let page = if self == Page::Spotify {
            Page::Music
        } else {
            self
        };
        Page::LIST.iter().position(|&p| p == page).unwrap_or(0)
    }
}

const SPOTIFY_INTRO: &str = "Play, pause, skip, covers and lyrics work with no setup. \
    Playlists and likes need your own free Spotify developer app: steps 1 to 4, about two \
    minutes.";

/// Who can use the library (Spotify's development-mode rules, checked
/// 2026-10-01: developer.spotify.com/documentation/web-api/concepts/quota-modes).
pub(super) const ELIGIBILITY: &str = "The account that makes the Spotify app needs Premium. At most 5 \
    accounts can use it, the owner included, each added under User Management.";

/// What to do when Spotify refuses a logged-in account (lava-1xk.26).
pub(super) const REFUSED: &str = "Spotify refused this account: the app's owner needs \
    Premium, and others must be added under User Management. Then disconnect and connect \
    again.";

/// The open settings screen: which page, whether the keys move between
/// pages or within one, and the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsView {
    pub page: Page,
    /// The keys work the page's rows (else the list of pages).
    pub in_rows: bool,
    pub cursor: usize,
    /// The first row shown; follows the cursor.
    pub top: usize,
}

/// One setting (or button) on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Style,
    Palette,
    Heat,
    Speed,
    Background,
    ColorRange,
    StripeFix,
    Face,
    Hour24,
    Seconds,
    FocusLength,
    ShortBreak,
    LongBreak,
    Cycles,
    Bell,
    /// Where widget `n` (in [`WIDGETS`]) goes.
    Place(usize),
    /// Where on the lamp widget `n` sits.
    Position(usize),
    Backing,
    /// The ink of text on the lamp.
    LampText,
    Spotify,
    CoverDetail,
    CoverSize,
    InlineCover,
    Mouse,
    LampOnly,
    HintLine,
    Smoothness,
    CornerClock,
    Reset(Page),
    // The Spotify setup.
    SetupStatus,
    Eligibility,
    Dashboard,
    CopyAddress,
    ClientId,
    Connect,
    CopyLoginLink,
    /// Where the login is kept: the Keychain or a private file.
    LoginStore,
    PlayerApp,
    /// Clear the saved lyrics and covers.
    ClearSaved,
}

/// How a row takes keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A value to step through: `←` `→`, enter steps on.
    Choice,
    /// Enter does it.
    Button,
    /// Enter to type (or paste) into it.
    Text,
    /// Just says something.
    Info,
}

/// A row as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub item: Item,
    pub label: String,
    pub value: String,
    /// One or two plain sentences: what it does.
    pub about: String,
    pub kind: Kind,
    /// Belongs to the row above (a widget's position): indented.
    pub sub: bool,
}

/// The settings screen's own state beyond the view: the Client ID being
/// typed, what the last button did, a pending confirmation.
#[derive(Debug, Default)]
pub struct SettingsState {
    /// The Client ID field's text.
    pub field: String,
    /// Typing into the field.
    pub editing: bool,
    /// What the field says about its text: `Err` a problem, `Ok` saved.
    pub note: Option<Result<String, String>>,
    /// The address was sent to the clipboard.
    pub copied: bool,
    /// The dashboard was opened.
    pub opened: bool,
    /// A reset or log-out waiting for its second enter.
    armed: Option<(Item, Instant)>,
}

/// A Spotify Client ID, cleaned up: 32 hex digits (spaces and line breaks
/// from a paste are dropped), or why it isn't one.
pub fn check_client_id(text: &str) -> Result<String, String> {
    let id: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if id.is_empty() {
        return Err("Paste the Client ID from your Spotify app's page.".into());
    }
    if !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(
            "That isn't a Client ID: it only has the digits 0-9 and the letters a-f.".into(),
        );
    }
    let n = id.chars().count();
    if n != 32 {
        return Err(format!(
            "A Client ID is 32 characters long; this one has {n}."
        ));
    }
    Ok(id.to_ascii_lowercase())
}

/// The plain name of a dock widget.
fn widget_name(i: usize) -> &'static str {
    match WIDGETS[i].name() {
        "clock" => "clock",
        "pomodoro" => "focus timer",
        "music" => "music player",
        "lyrics" => "lyrics",
        "cover" => "album cover",
        other => other,
    }
}

fn place_name(place: Place) -> &'static str {
    match place {
        Place::Side => "beside the lamp",
        Place::Overlay => "on the lamp",
        Place::Off => "off",
    }
}

const PLACES: [Place; 3] = [Place::Side, Place::Overlay, Place::Off];

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// `i` moved `step` through `n` options, wrapping.
fn wrap(i: usize, n: usize, step: isize) -> usize {
    (i as isize + step).rem_euclid(n as isize) as usize
}

/// The next value of `list` above (or below) `current`, if any.
fn step_in(list: &[u32], current: u32, up: bool) -> Option<u32> {
    if up {
        list.iter().copied().find(|&v| v > current)
    } else {
        list.iter().rev().copied().find(|&v| v < current)
    }
}

fn index_of<T: PartialEq>(all: &[T], value: &T) -> usize {
    all.iter().position(|v| v == value).unwrap_or(0)
}

const COLOR_RANGES: [ColorChoice; 5] = [
    ColorChoice::Auto,
    ColorChoice::Truecolor,
    ColorChoice::Ansi256,
    ColorChoice::Ansi16,
    ColorChoice::None,
];
const STRIPE_FIXES: [CellsChoice; 4] = [
    CellsChoice::Auto,
    CellsChoice::Opaque,
    CellsChoice::Translucent,
    CellsChoice::Background,
];
const COVER_SIZES: [CoverSize; 4] = [
    CoverSize::Small,
    CoverSize::Medium,
    CoverSize::Large,
    CoverSize::Fill,
];

fn color_range_name(c: ColorChoice) -> &'static str {
    match c {
        ColorChoice::Auto => "automatic",
        ColorChoice::Truecolor => "millions",
        ColorChoice::Ansi256 => "256 colours",
        ColorChoice::Ansi16 => "16 colours",
        ColorChoice::None => "no colour",
    }
}

fn stripe_fix_name(c: CellsChoice) -> &'static str {
    match c {
        CellsChoice::Auto => "automatic",
        CellsChoice::Opaque => "off",
        CellsChoice::Translucent => "see-through window",
        CellsChoice::Background => "lines between rows",
    }
}

fn cover_size_name(s: CoverSize) -> &'static str {
    match s {
        CoverSize::Small => "small",
        CoverSize::Medium => "medium",
        CoverSize::Large => "large",
        CoverSize::Fill => "as big as fits",
    }
}

impl Model {
    /// The open settings screen, if it is.
    pub fn settings_view(&self) -> Option<SettingsView> {
        match self.overlay {
            Overlay::Settings(view) => Some(view),
            _ => None,
        }
    }

    /// The Spotify setup is on screen: the player and the library stay
    /// connected for it, so it can say how they are.
    pub fn spotify_setup_open(&self) -> bool {
        self.settings_view()
            .is_some_and(|v| v.page == Page::Spotify)
    }

    /// `,`: open on the first page's list.
    pub(super) fn open_settings(&mut self) {
        self.open_settings_at(Page::Look, false);
    }

    /// Open on `page`, in its rows or in the list of pages.
    pub(super) fn open_settings_at(&mut self, page: Page, in_rows: bool) {
        self.music.keys = false;
        self.settings_screen.editing = false;
        self.settings_screen.armed = None;
        self.overlay = Overlay::Settings(SettingsView {
            page,
            in_rows,
            cursor: 0,
            top: 0,
        });
    }

    fn close_settings(&mut self) {
        self.settings_screen.editing = false;
        self.settings_screen.armed = None;
        self.overlay = Overlay::None;
    }

    /// The items on `page`, in order.
    fn items(&self, page: Page) -> Vec<Item> {
        use Item::*;
        let widget = |name: &str| dock::by_name(name).map_or(0, |(i, _)| i);
        let with_position = |i: usize, out: &mut Vec<Item>| {
            out.push(Place(i));
            if self.settings.dock.place(WIDGETS[i]) == dock::Place::Overlay {
                out.push(Position(i));
            }
        };
        let mut out = Vec::new();
        match page {
            Page::Look => out.extend([
                Style, Palette, Heat, Speed, Background, ColorRange, StripeFix,
            ]),
            Page::Clock => out.extend([
                Face,
                Hour24,
                Seconds,
                FocusLength,
                ShortBreak,
                LongBreak,
                Cycles,
                Bell,
            ]),
            Page::Widgets => {
                for i in 0..WIDGETS.len() {
                    with_position(i, &mut out);
                }
                out.extend([LampText, Backing]);
            }
            Page::Music => {
                out.push(Spotify);
                with_position(widget("lyrics"), &mut out);
                out.extend([CoverDetail, CoverSize, InlineCover, ClearSaved]);
            }
            Page::Controls => out.push(Mouse),
            Page::Window => out.extend([LampOnly, HintLine, Smoothness, CornerClock]),
            Page::Spotify => {
                out.extend([
                    SetupStatus,
                    Eligibility,
                    Dashboard,
                    CopyAddress,
                    ClientId,
                    Connect,
                ]);
                if self.library.account() == Account::LoggingIn {
                    out.push(CopyLoginLink);
                }
                out.extend([LoginStore, PlayerApp]);
            }
        }
        if page != Page::Spotify {
            out.push(Reset(page));
        }
        out
    }

    /// The rows of `page`, as drawn.
    pub fn settings_rows(&self, page: Page) -> Vec<Row> {
        self.items(page).into_iter().map(|i| self.row(i)).collect()
    }

    fn row(&self, item: Item) -> Row {
        let s = &self.settings;
        let row = |label: &str, value: String, about: &str, kind: Kind| Row {
            item,
            label: label.into(),
            value,
            about: about.into(),
            kind,
            sub: false,
        };
        let choice =
            |label: &str, value: &str, about: &str| row(label, value.into(), about, Kind::Choice);
        match item {
            Item::Style => choice("style", self.style.style().name(), "How the wax is drawn."),
            Item::Palette => choice(
                "colours",
                self.theme.palette().name,
                "The colours of the wax, the liquid and the text.",
            ),
            Item::Heat => {
                let heat = usize::from(self.world.heat());
                let levels = usize::from(*HEAT_LEVELS.end());
                let bar = "▮".repeat(heat) + &"▯".repeat(levels - heat);
                row(
                    "heat",
                    bar,
                    "More heat makes more wax rise, and faster.",
                    Kind::Choice,
                )
            }
            Item::Speed => row(
                "speed",
                format!("×{}", self.speed.factor()),
                "How fast the wax moves. ×1 is normal.",
                Kind::Choice,
            ),
            Item::Background => choice(
                "background",
                if s.theme.transparent {
                    "see-through"
                } else {
                    "painted"
                },
                "See-through leaves the background to your terminal, so its own colour \
                 or transparency shows.",
            ),
            Item::ColorRange => choice(
                "colour range",
                color_range_name(s.display.color),
                "How many colours to use. Leave it on automatic unless colours look wrong.",
            ),
            Item::StripeFix => choice(
                "stripe fix",
                stripe_fix_name(s.display.cells),
                "Stops thin stripes in the wax. See-through window: for a see-through \
                 terminal window (Ghostty with background opacity). Lines between rows: for \
                 dark lines between the rows of wax (macOS Terminal, Ghostex). Automatic picks for \
                 your terminal.",
            ),
            Item::Face => choice(
                "clock face",
                self.face.name(),
                "How the clock shows the time.",
            ),
            Item::Hour24 => choice(
                "time format",
                if s.clock.hour24 { "24-hour" } else { "12-hour" },
                "14:30 or 2:30.",
            ),
            Item::Seconds => choice(
                "seconds",
                on_off(s.clock.seconds),
                "Shows the seconds when the clock is big. Off: just hours and minutes.",
            ),
            Item::FocusLength => row(
                "focus length",
                format!("{} min", s.pomodoro.focus_min),
                "How long each focus session lasts. Space starts the timer.",
                Kind::Choice,
            ),
            Item::ShortBreak => row(
                "short break",
                format!("{} min", s.pomodoro.short_break_min),
                "The break after each focus session.",
                Kind::Choice,
            ),
            Item::LongBreak => row(
                "long break",
                format!("{} min", s.pomodoro.long_break_min),
                "The longer break after a few focus sessions.",
                Kind::Choice,
            ),
            Item::Cycles => row(
                "long break after",
                match s.pomodoro.cycles {
                    1 => "every session".into(),
                    n => format!("{n} sessions"),
                },
                "How many focus sessions come before a long break.",
                Kind::Choice,
            ),
            Item::Bell => choice(
                "sound at the end",
                on_off(s.pomodoro.bell),
                "Rings your terminal's bell when a focus session or a break ends.",
            ),
            Item::Place(i) => {
                let about = match WIDGETS[i].name() {
                    "lyrics" => {
                        "Lyrics come from lrclib.net. While they're on, the song's title, \
                         artist, album and length are sent there."
                    }
                    "cover" => "The playing song's album cover.",
                    "music" => "What's playing in Spotify, with buttons to control it.",
                    "pomodoro" => "The focus timer: work, then take a break.",
                    _ => "The time of day.",
                };
                choice(widget_name(i), place_name(s.dock.place(WIDGETS[i])), about)
            }
            Item::Position(i) => Row {
                sub: true,
                ..choice(
                    "position",
                    s.dock.anchor(WIDGETS[i]).name(),
                    "Where on the lamp it sits.",
                )
            },
            Item::Backing => choice(
                "behind things on the lamp",
                match s.dock.backing {
                    Backing::None => "nothing",
                    Backing::Soft => "soft shade",
                },
                "What items on the lamp sit on. The soft shade needs millions of colours.",
            ),
            Item::LampText => choice(
                "text on the lamp",
                match s.dock.text {
                    TextInk::Auto => "automatic",
                    TextInk::Light => "light",
                    TextInk::Dark => "dark",
                },
                "Automatic makes each letter light or dark, whichever stands out from the \
                 wax behind it. Light or dark keeps it one colour, even where that's hard \
                 to read.",
            ),
            Item::Spotify => row(
                "spotify",
                self.spotify_status().into(),
                "Play, pause and skip work with the Spotify app on its own. Playlists and \
                 likes need a one-time setup: press enter.",
                Kind::Button,
            ),
            Item::CoverDetail => choice(
                "cover picture",
                s.art.detail.label(),
                "Auto picks the best this terminal can show. Photo needs one that shows \
                 images, like kitty or Ghostty. Fine, medium and coarse are drawn in \
                 text; coarser looks softer.",
            ),
            Item::CoverSize => choice(
                "cover size",
                cover_size_name(s.art.size),
                "The largest the album cover gets.",
            ),
            Item::InlineCover => choice(
                "small cover with music",
                on_off(s.art.inline),
                "A little cover beside the song, while the album cover itself is off.",
            ),
            Item::Mouse => choice(
                "mouse",
                on_off(s.input.mouse),
                if self.option_drag {
                    "Click buttons, lists and the wax. While it's on, hold Option to \
                     select text."
                } else {
                    "Click buttons, lists and the wax. While it's on, hold Shift to \
                     select text."
                },
            ),
            Item::LampOnly => choice(
                "lamp only",
                on_off(self.minimal()),
                "Hides everything but the lamp and a small clock. Press m to switch at any \
                 time.",
            ),
            Item::HintLine => choice(
                "hint line",
                if s.ui.status_bar { "shown" } else { "hidden" },
                "The line of key hints along the bottom. Press b to switch.",
            ),
            Item::Smoothness => row(
                "smoothness",
                format!("{} frames a second", s.display.fps),
                "How often the lamp is redrawn. Lower saves battery; 60 is smooth.",
                Kind::Choice,
            ),
            Item::CornerClock => choice(
                "lamp-only clock",
                match s.minimal.clock {
                    MinimalClock::Corner => "in the corner",
                    MinimalClock::Off => "hidden",
                },
                "The small clock shown while only the lamp is.",
            ),
            Item::Reset(page) => {
                let armed = self.armed(item);
                let about = if page == Page::Music {
                    "Puts this page back to how it started. Your Spotify setup stays."
                } else {
                    "Puts this page back to how it started."
                };
                row(
                    "reset this page",
                    if armed { "press enter again" } else { "reset" }.into(),
                    about,
                    Kind::Button,
                )
            }
            Item::SetupStatus => row(
                "status",
                self.spotify_status().into(),
                if self.library.refused.is_some() {
                    REFUSED
                } else {
                    SPOTIFY_INTRO
                },
                Kind::Info,
            ),
            Item::Eligibility => row(
                "before you start",
                "Premium needed".into(),
                ELIGIBILITY,
                Kind::Info,
            ),
            Item::Dashboard => row(
                "1  make a spotify app",
                if self.settings_screen.opened {
                    "opened"
                } else {
                    "open website"
                }
                .into(),
                "Opens developer.spotify.com/dashboard in your browser. Log in, press \
                 Create app, and give it any name and description.",
                Kind::Button,
            ),
            Item::CopyAddress => row(
                "2  add this address",
                if self.settings_screen.copied {
                    "copied"
                } else {
                    "copy"
                }
                .into(),
                &format!(
                    "In the app's form, paste {REDIRECT_URI} under Redirect URIs and press \
                     Add. Tick Web API, accept the terms and save. If pasting doesn't work, \
                     type it in exactly."
                ),
                Kind::Button,
            ),
            Item::ClientId => self.client_id_row(),
            Item::Connect => self.connect_row(),
            Item::CopyLoginLink => row(
                "copy the login link",
                "copy".into(),
                "No browser tab opened? Copy the link and open it in your browser.",
                Kind::Button,
            ),
            Item::LoginStore => choice(
                "keep the login in",
                // The same words everywhere (the about names the Keychain).
                match s.spotify.store {
                    LoginStore::System => "password store",
                    LoginStore::File => "private file",
                },
                if cfg!(target_os = "macos") {
                    "The Keychain is safest, but macOS asks before LavaTUI uses it, and \
                     again after each update: choose Always Allow. A private file never \
                     asks, but any program you run can read it."
                } else {
                    "The system's password store is safest. A private file works where \
                     there is none, but any program you run can read it."
                },
            ),
            Item::PlayerApp => self.player_row(),
            Item::ClearSaved => self.saved_row(),
        }
    }

    fn saved_row(&self) -> Row {
        let files = &self.saved_files;
        let value = match files.saved.map(|s| s.total()) {
            _ if self.armed(Item::ClearSaved) => "press enter again".into(),
            _ if !files.exist() => "nothing saved".into(),
            None => "…".into(),
            Some(total) if total.files == 0 => "nothing saved".into(),
            Some(total) => format!("{} · clear", size_words(total.bytes)),
        };
        let sizes = match files.saved {
            Some(s) if s.total().files > 0 => format!(
                " Right now: {} of lyrics and {} of covers.",
                size_words(s.lyrics.bytes),
                size_words(s.covers.bytes)
            ),
            _ => String::new(),
        };
        let about = if files.exist() {
            format!(
                "Lyrics and album covers are kept on this computer, so they show up fast \
                 and work offline. They stay small: old ones are removed on their own.\
                 {sizes} Press enter twice to clear saved lyrics and covers."
            )
        } else {
            "Nothing is kept on this computer.".into()
        };
        Row {
            item: Item::ClearSaved,
            label: "saved lyrics & covers".into(),
            value,
            about,
            kind: Kind::Button,
            sub: false,
        }
    }

    /// Pick up the saved files' sizes (and a finished clear); keep them
    /// measured while the music page is open.
    pub(super) fn sync_saved_files(&mut self, now: Instant) {
        if self.saved_files.poll() {
            self.toast("saved lyrics and covers cleared");
        }
        if self.settings_view().is_some_and(|v| v.page == Page::Music) {
            self.saved_files.measure(now);
        }
    }

    fn client_id_row(&self) -> Row {
        let state = &self.settings_screen;
        let saved = self.settings.spotify.client_id.trim();
        let value = if state.editing {
            format!("{}▏", state.field)
        } else if !saved.is_empty() {
            saved.to_owned()
        } else if crate::spotify_web::client_id_from_env().is_some() {
            "set outside the app".into()
        } else {
            "paste here".into()
        };
        let about = match &state.note {
            Some(Err(problem)) => problem.clone(),
            Some(Ok(done)) => done.clone(),
            None if state.editing => {
                "Paste the Client ID, then press enter to save it (esc to stop). \
                 Leave it empty to remove it."
                    .into()
            }
            None => "The Client ID names the app you just made. Copy it from the app's page, \
                     press enter here and paste it. Never paste the Client secret."
                .into(),
        };
        Row {
            item: Item::ClientId,
            label: "3  paste the client id".into(),
            value,
            about,
            kind: Kind::Text,
            sub: false,
        }
    }

    fn connect_row(&self) -> Row {
        let lib = &self.library;
        let (value, about): (&str, String) = match lib.account() {
            _ if self.settings.spotify_client_id().is_none() => (
                "connect",
                "Do steps 1 to 3 first, then connect here.".into(),
            ),
            Account::Unavailable => ("connect", "Connecting to Spotify…".into()),
            Account::LoggedOut => (
                "connect",
                match &lib.login_error {
                    Some(e) => format!(
                        "That didn't work: {e}. Check the address in step 2 matches exactly, \
                         then try again."
                    ),
                    None => "Opens Spotify in your browser to allow access. When it says \
                             you can close the tab, come back here."
                        .into(),
                },
            ),
            Account::LoggingIn => (
                "waiting for browser",
                "Allow access in your browser, then come back. Press enter to cancel.".into(),
            ),
            Account::LoggedIn if lib.refused.is_some() => (
                if self.armed(Item::Connect) {
                    "press enter again"
                } else {
                    "refused"
                },
                REFUSED.into(),
            ),
            Account::LoggedIn => {
                let name = lib
                    .me
                    .as_ref()
                    .map(|u| format!("Connected as {}. ", u.name()))
                    .unwrap_or_else(|| "Connected. ".into());
                (
                    if self.armed(Item::Connect) {
                        "press enter again"
                    } else {
                        "connected"
                    },
                    format!(
                        "{name}Press b with the music keys (A) for your playlists. \
                         Enter twice here disconnects."
                    ),
                )
            }
        };
        Row {
            item: Item::Connect,
            label: "4  connect".into(),
            value: value.into(),
            about,
            kind: Kind::Button,
            sub: false,
        }
    }

    fn player_row(&self) -> Row {
        let snap = self.music.snapshot.as_ref();
        let status = snap.map(|s| &s.status);
        let (value, about) = match status {
            None | Some(Status::Connecting) => ("checking", "Looking for the Spotify app…".into()),
            Some(Status::Playing) => ("playing", "The Spotify app is playing.".into()),
            Some(Status::Paused | Status::Stopped) => {
                ("open", "The Spotify app is open and ready.".into())
            }
            Some(Status::Unavailable(why)) => {
                let message = snap
                    .and_then(|s| s.unavailable_message())
                    .unwrap_or_default();
                match why {
                    Unavailable::NotRunning => (
                        "not running",
                        format!("{message}. Open the Spotify app; LavaTUI never starts it."),
                    ),
                    Unavailable::NotInstalled => (
                        "not installed",
                        format!("{message}. Install the Spotify app to play music."),
                    ),
                    Unavailable::PermissionDenied => ("needs permission", format!("{message}.")),
                    _ => ("not answering", format!("{message}.")),
                }
            }
        };
        Row {
            item: Item::PlayerApp,
            label: "spotify app".into(),
            value: value.into(),
            about,
            kind: Kind::Info,
            sub: false,
        }
    }

    /// One or two words for where the Spotify setup stands.
    fn spotify_status(&self) -> &'static str {
        if self.library.refused.is_some() {
            return "refused";
        }
        if self.settings.spotify_client_id().is_none() {
            return "not set up";
        }
        match self.library.account() {
            Account::Unavailable => "set up",
            Account::LoggedOut => "not connected",
            Account::LoggingIn => "waiting for browser",
            Account::LoggedIn => "connected",
        }
    }

    fn armed(&self, item: Item) -> bool {
        self.settings_screen
            .armed
            .is_some_and(|(i, at)| i == item && self.now - at < CONFIRM_WINDOW)
    }

    // --- keys and clicks ---------------------------------------------------

    /// Keys and clicks while the settings screen is open. Returns whether
    /// the action was its own.
    pub(super) fn settings_action(&mut self, mut view: SettingsView, action: Action) -> bool {
        let now = self.now;
        if self.settings_screen.editing {
            return self.field_action(action, now);
        }
        let n = self.items(view.page).len();
        match (view.in_rows, action) {
            (_, Action::Close | Action::Settings) => {
                self.close_settings();
                return true;
            }
            (_, Action::Click { col, row }) => return self.settings_click(view, col, row, now),
            (false, Action::Up | Action::Down | Action::SwitchPage(_)) => {
                let down = matches!(action, Action::Down | Action::SwitchPage(true));
                let i = wrap(
                    view.page.index(),
                    Page::LIST.len(),
                    if down { 1 } else { -1 },
                );
                view = SettingsView {
                    page: Page::LIST[i],
                    cursor: 0,
                    top: 0,
                    ..view
                };
            }
            (false, Action::Edge(end)) => {
                view.page = Page::LIST[if end { Page::LIST.len() - 1 } else { 0 }];
            }
            (false, Action::Keep | Action::Change(true)) => {
                view.in_rows = true;
                view.cursor = 0;
                view.top = 0;
            }
            (false, Action::Back) => {
                self.close_settings();
                return true;
            }
            (true, Action::Up) => view.cursor = wrap(view.cursor, n, -1),
            (true, Action::Down) => view.cursor = wrap(view.cursor, n, 1),
            (true, Action::Page(down)) => {
                let step = geometry::visible_rows(self.layout.area, self, &view).max(1);
                view.cursor = if down {
                    (view.cursor + step).min(n - 1)
                } else {
                    view.cursor.saturating_sub(step)
                };
            }
            (true, Action::Edge(end)) => view.cursor = if end { n - 1 } else { 0 },
            (true, Action::SwitchPage(down)) => {
                let i = wrap(
                    view.page.index(),
                    Page::LIST.len(),
                    if down { 1 } else { -1 },
                );
                view = SettingsView {
                    page: Page::LIST[i],
                    in_rows: true,
                    cursor: 0,
                    top: 0,
                };
            }
            (true, Action::Change(up)) => {
                let item = self.items(view.page)[view.cursor.min(n - 1)];
                if self.row(item).kind == Kind::Choice {
                    self.change(item, up, now);
                }
            }
            (true, Action::Keep) => {
                let item = self.items(view.page)[view.cursor.min(n - 1)];
                view = self.activate(view, item, now);
            }
            (true, Action::Back) if view.page == Page::Spotify => view = self.leave_setup(),
            (true, Action::Back) => {
                view.in_rows = false;
                self.settings_screen.armed = None;
            }
            _ => return false,
        }
        self.show_settings(view);
        true
    }

    /// Put `view` up, its cursor on a row that exists and in sight.
    fn show_settings(&mut self, mut view: SettingsView) {
        // Leaving the overlay (a key closed it) wins.
        if self.settings_view().is_none() {
            return;
        }
        let n = self.items(view.page).len();
        view.cursor = view.cursor.min(n.saturating_sub(1));
        let rows = geometry::visible_rows(self.layout.area, self, &view);
        view.top = crate::ui::picker::visible_top(view.top, view.cursor, rows, n);
        self.overlay = Overlay::Settings(view);
    }

    /// From the Spotify setup back to `music & lyrics`, on its row.
    fn leave_setup(&mut self) -> SettingsView {
        self.settings_screen.note = None;
        self.settings_screen.armed = None;
        let cursor = self
            .items(Page::Music)
            .iter()
            .position(|&i| i == Item::Spotify)
            .unwrap_or(0);
        SettingsView {
            page: Page::Music,
            in_rows: true,
            cursor,
            top: 0,
        }
    }

    fn settings_click(&mut self, mut view: SettingsView, col: u16, row: u16, now: Instant) -> bool {
        match geometry::hit(self.layout.area, self, &view, col, row) {
            Some(Hit::Page(page)) => {
                view = SettingsView {
                    page,
                    in_rows: false,
                    cursor: 0,
                    top: 0,
                };
            }
            Some(Hit::Row(i)) => {
                if view.in_rows && view.cursor == i {
                    let item = self.items(view.page)[i];
                    view = self.activate(view, item, now);
                } else {
                    view.in_rows = true;
                    view.cursor = i;
                }
            }
            Some(Hit::Back) => {
                return self.settings_action(view, Action::Back);
            }
            // Off the screen's rows: swallowed.
            None => return true,
        }
        self.show_settings(view);
        true
    }

    /// Enter on `item`: a choice steps on, a button does its thing.
    fn activate(&mut self, view: SettingsView, item: Item, now: Instant) -> SettingsView {
        if self.row(item).kind == Kind::Choice {
            self.change(item, true, now);
            return view;
        }
        match item {
            Item::Spotify => {
                return SettingsView {
                    page: Page::Spotify,
                    in_rows: true,
                    cursor: 0,
                    top: 0,
                };
            }
            Item::Reset(page) => {
                if self.armed(item) {
                    self.settings_screen.armed = None;
                    self.reset_page(page, now);
                } else {
                    self.settings_screen.armed = Some((item, now));
                }
            }
            Item::ClearSaved => {
                if self.armed(item) {
                    self.settings_screen.armed = None;
                    self.saved_files.clear();
                } else if self.saved_files.exist() {
                    self.settings_screen.armed = Some((item, now));
                }
            }
            Item::Dashboard => {
                open_in_browser(DASHBOARD_URL);
                self.settings_screen.opened = true;
            }
            Item::CopyAddress => {
                self.copy = Some(REDIRECT_URI.into());
                self.settings_screen.copied = true;
            }
            Item::CopyLoginLink => {
                if let Some(url) = &self.library.login_url {
                    self.copy = Some(url.clone());
                    self.toast("login link copied");
                }
            }
            Item::ClientId => {
                let state = &mut self.settings_screen;
                state.editing = true;
                state.note = None;
                state.field = self.settings.spotify.client_id.clone();
            }
            Item::Connect => self.connect_key(now),
            _ => {}
        }
        view
    }

    /// Step 4: connect (log in in the browser), cancel a login waiting on
    /// the browser, or, twice, disconnect.
    fn connect_key(&mut self, now: Instant) {
        if self.settings.spotify_client_id().is_none() {
            return;
        }
        // The client starts with the setup open; make sure it has.
        self.sync_library();
        if self.library.locked() || self.library.pending.is_some() {
            // Reading the saved login first (the setup asked for it).
            return;
        }
        let lib = &mut self.library;
        match lib.account() {
            Account::Unavailable => {}
            Account::LoggingIn => lib.cancel_login(),
            Account::LoggedOut => {
                lib.start_login();
            }
            Account::LoggedIn => {
                if self.armed(Item::Connect) {
                    self.settings_screen.armed = None;
                    self.library.logout();
                } else {
                    self.settings_screen.armed = Some((Item::Connect, now));
                }
            }
        }
    }

    /// Keys while typing the Client ID.
    fn field_action(&mut self, action: Action, now: Instant) -> bool {
        let state = &mut self.settings_screen;
        match action {
            Action::Type(c) if state.field.chars().count() < FIELD_MAX => {
                state.field.push(c);
                state.note = None;
            }
            Action::Type(_) => {}
            Action::Erase => {
                state.field.pop();
                state.note = None;
            }
            Action::Keep => self.save_client_id(now),
            Action::Back | Action::Close => {
                state.editing = false;
                state.note = None;
            }
            // Clicks and the rest stop the typing, then do their thing.
            Action::Click { .. } => {
                state.editing = false;
                return false;
            }
            _ => return action != Action::Quit,
        }
        true
    }

    /// Pasted text (bracketed paste): into the Client ID field when the
    /// Spotify setup is open; ignored anywhere else.
    pub fn paste(&mut self, text: &str, now: Instant) {
        self.now = now;
        let Some(view) = self.settings_view().filter(|v| v.page == Page::Spotify) else {
            return;
        };
        let state = &mut self.settings_screen;
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        state.field = clean.trim().chars().take(FIELD_MAX).collect();
        state.editing = true;
        state.note = check_client_id(&state.field).err().map(Err);
        if state.note.is_none() {
            state.note = Some(Ok("Looks right. Press enter to save it.".into()));
        }
        let cursor = self
            .items(Page::Spotify)
            .iter()
            .position(|&i| i == Item::ClientId)
            .unwrap_or(0);
        self.show_settings(SettingsView {
            in_rows: true,
            cursor,
            ..view
        });
    }

    /// Enter in the field: save a good Client ID (or remove it when empty)
    /// and move on to connecting.
    fn save_client_id(&mut self, now: Instant) {
        let field = self.settings_screen.field.trim().to_owned();
        let id = if field.is_empty() {
            String::new()
        } else {
            match check_client_id(&field) {
                Ok(id) => id,
                Err(problem) => {
                    self.settings_screen.note = Some(Err(problem));
                    return;
                }
            }
        };
        let state = &mut self.settings_screen;
        state.editing = false;
        state.note = Some(Ok(if id.is_empty() {
            "Removed.".into()
        } else {
            "Saved. Now connect (step 4).".into()
        }));
        if id != self.settings.spotify.client_id {
            self.settings.spotify.client_id = id;
            self.library
                .set_client_id(self.settings.spotify_client_id());
            self.changed(now);
        }
        if let Some(view) = self.settings_view() {
            let next = self
                .items(view.page)
                .iter()
                .position(|&i| i == Item::Connect)
                .unwrap_or(view.cursor);
            let cursor = if self.settings.spotify.client_id.is_empty() {
                view.cursor
            } else {
                next
            };
            self.show_settings(SettingsView { cursor, ..view });
        }
    }

    // --- changing settings -------------------------------------------------

    /// `←` (`up` false) or `→` on a choice: its previous or next value,
    /// live and saved.
    fn change(&mut self, item: Item, up: bool, now: Instant) {
        let step = if up { 1 } else { -1 };
        let s = &mut self.settings;
        match item {
            Item::Style | Item::Palette | Item::Face => {
                let kind = match item {
                    Item::Style => PickerKind::Style,
                    Item::Palette => PickerKind::Palette,
                    _ => PickerKind::Face,
                };
                let i = wrap(self.current(kind), kind.items().len(), step);
                self.apply_pick(kind, i);
                self.persist_pick(kind);
                return;
            }
            Item::Heat => {
                let heat = if up {
                    self.world.heat().saturating_add(1)
                } else {
                    self.world.heat().saturating_sub(1)
                };
                self.world.set_heat(heat);
                self.settings.lamp.heat = self.world.heat();
            }
            Item::Speed => {
                self.speed = if up {
                    self.speed.faster()
                } else {
                    self.speed.slower()
                };
                self.settings.lamp.speed = self.speed.factor();
            }
            Item::Background => s.theme.transparent = !s.theme.transparent,
            Item::ColorRange => {
                let i = wrap(
                    index_of(&COLOR_RANGES, &s.display.color),
                    COLOR_RANGES.len(),
                    step,
                );
                s.display.color = COLOR_RANGES[i];
                self.overridden.retain(|o| *o != Overridden::Color);
                self.theme = Theme::new(self.theme.palette(), depth_for(s.display.color));
            }
            Item::StripeFix => {
                let i = wrap(
                    index_of(&STRIPE_FIXES, &s.display.cells),
                    STRIPE_FIXES.len(),
                    step,
                );
                s.display.cells = STRIPE_FIXES[i];
            }
            Item::Hour24 => s.clock.hour24 = !s.clock.hour24,
            Item::Seconds => s.clock.seconds = !s.clock.seconds,
            Item::FocusLength | Item::ShortBreak | Item::LongBreak | Item::Cycles => {
                let p = &mut s.pomodoro;
                let (value, list): (&mut u32, &[u32]) = match item {
                    Item::FocusLength => (&mut p.focus_min, MINUTES),
                    Item::ShortBreak => (&mut p.short_break_min, MINUTES),
                    Item::LongBreak => (&mut p.long_break_min, MINUTES),
                    _ => (&mut p.cycles, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]),
                };
                match step_in(list, *value, up) {
                    Some(v) => *value = v,
                    None => return,
                }
                self.pomodoro.set_config(pomodoro_config(&self.settings));
            }
            Item::Bell => s.pomodoro.bell = !s.pomodoro.bell,
            Item::Place(i) => {
                let widget = WIDGETS[i];
                let place = PLACES[wrap(index_of(&PLACES, &s.dock.place(widget)), 3, step)];
                s.dock.set(widget, place);
                if place == Place::Overlay {
                    self.lava_focus = Some(i);
                }
                self.sync_music();
            }
            Item::Position(i) => {
                let widget = WIDGETS[i];
                let at = index_of(&Anchor::ALL, &s.dock.anchor(widget));
                s.dock
                    .set_anchor(widget, Anchor::ALL[wrap(at, Anchor::ALL.len(), step)]);
                self.lava_focus = Some(i);
            }
            Item::Backing => {
                s.dock.backing = match s.dock.backing {
                    Backing::None => Backing::Soft,
                    Backing::Soft => Backing::None,
                }
            }
            Item::LampText => {
                let i = wrap(
                    index_of(&TextInk::ALL, &s.dock.text),
                    TextInk::ALL.len(),
                    step,
                );
                s.dock.text = TextInk::ALL[i];
            }
            Item::CoverDetail => {
                let i = wrap(
                    index_of(&Detail::ALL, &s.art.detail),
                    Detail::ALL.len(),
                    step,
                );
                s.art.detail = Detail::ALL[i];
            }
            Item::CoverSize => {
                let i = wrap(index_of(&COVER_SIZES, &s.art.size), COVER_SIZES.len(), step);
                s.art.size = COVER_SIZES[i];
            }
            Item::InlineCover => s.art.inline = !s.art.inline,
            Item::Mouse => s.input.mouse = !s.input.mouse,
            Item::LoginStore => {
                s.spotify.store = match s.spotify.store {
                    LoginStore::System => LoginStore::File,
                    LoginStore::File => LoginStore::System,
                };
                // Moving it reads it first: macOS may ask.
                if self.library.locked() {
                    self.toast(super::library::KEYCHAIN_HEADS_UP);
                }
                self.library.set_store(self.settings.spotify.store);
            }
            Item::LampOnly => {
                s.ui.mode = match s.ui.mode {
                    UiMode::Full => UiMode::Minimal,
                    UiMode::Minimal => UiMode::Full,
                };
                self.overridden.retain(|o| *o != Overridden::Mode);
            }
            Item::HintLine => s.ui.status_bar = !s.ui.status_bar,
            Item::Smoothness => {
                match step_in(FPS, s.display.fps, up) {
                    Some(v) => s.display.fps = v,
                    None => return,
                }
                self.overridden.retain(|o| *o != Overridden::Fps);
            }
            Item::CornerClock => {
                s.minimal.clock = match s.minimal.clock {
                    MinimalClock::Corner => MinimalClock::Off,
                    MinimalClock::Off => MinimalClock::Corner,
                }
            }
            _ => return,
        }
        self.changed(now);
    }

    /// `reset this page`: the page's settings back to their defaults,
    /// live. The Spotify Client ID is never reset.
    fn reset_page(&mut self, page: Page, now: Instant) {
        let d = Settings::default();
        let s = &mut self.settings;
        match page {
            Page::Look => {
                s.lamp = d.lamp;
                s.theme = d.theme;
                s.display.color = d.display.color;
                s.display.cells = d.display.cells;
                self.overridden.retain(|o| {
                    !matches!(
                        o,
                        Overridden::Style | Overridden::Palette | Overridden::Color
                    )
                });
            }
            Page::Clock => {
                s.clock = d.clock;
                s.pomodoro = d.pomodoro;
            }
            Page::Widgets => s.dock = d.dock,
            Page::Music => {
                s.art = d.art;
                if let Some((_, widget)) = dock::by_name("lyrics") {
                    s.dock.set(widget, d.dock.place(widget));
                    s.dock.set_anchor(widget, d.dock.anchor(widget));
                }
            }
            Page::Controls => s.input = d.input,
            Page::Window => {
                s.ui = d.ui;
                s.display.fps = d.display.fps;
                s.minimal = d.minimal;
                self.overridden
                    .retain(|o| !matches!(o, Overridden::Mode | Overridden::Fps));
            }
            Page::Spotify => return,
        }
        self.apply_settings();
        self.toast(format!("{} reset", page.title()));
        self.changed(now);
    }

    /// Make the live lamp, clock and timer match the settings again.
    fn apply_settings(&mut self) {
        let s = &self.settings;
        self.style = StyleId::by_name(&s.lamp.style).unwrap_or_default();
        self.theme = Theme::new(palette_named(&s.theme.palette), depth_for(s.display.color));
        self.face = clock::face_by_name(&s.clock.face).unwrap_or_else(clock::default_face);
        self.world.set_heat(s.lamp.heat);
        self.speed = SimSpeed::from_factor(s.lamp.speed);
        self.pomodoro.set_config(pomodoro_config(s));
        self.sync_music();
    }
}

/// Open `url` in the browser, off the input path (it starts a process).
fn open_in_browser(url: &'static str) {
    #[cfg(not(test))]
    std::thread::spawn(move || {
        crate::thread_qos::worker();
        let _ = webbrowser::open(url);
    });
    #[cfg(test)]
    let _ = url;
}
