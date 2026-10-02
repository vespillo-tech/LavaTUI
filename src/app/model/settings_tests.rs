//! The settings screen's behaviour (lava-1xk.17): keys move and change,
//! changes are live and persist, reset, the Client ID field (typing,
//! paste, checking) and the Spotify connect flow against `FakeWeb`.

use std::path::PathBuf;

use super::settings_screen::check_client_id;
use super::*;
use crate::config::store::Store;
use crate::dock::Anchor;
use crate::spotify_web::fake::FakeWeb;
use crate::spotify_web::{REDIRECT_URI, Web};
use crate::ui::keymap::Action;

const ID: &str = "0123456789abcdef0123456789abcdef";

fn temp_config(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lavatui-settings-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("config.toml")
}

fn local() -> LocalTime {
    LocalTime {
        time: ClockTime::new(14, 32, 7).unwrap(),
        date: "thu 1 oct".into(),
        wall: SystemTime::UNIX_EPOCH,
    }
}

fn model_at(path: PathBuf) -> (Model, Instant) {
    let t0 = Instant::now();
    let m = Model::new(
        &Session::default(),
        Store::new(Some(path)),
        Rect::new(0, 0, 80, 24),
        None,
        local(),
        1,
        t0,
    );
    (m, t0)
}

/// Into the rows of `page` (by its place in the list).
fn open(m: &mut Model, t: Instant, page: usize) {
    m.update(Action::Settings, t);
    for _ in 0..page {
        m.update(Action::Down, t);
    }
    m.update(Action::Keep, t);
}

/// The cursor onto `item` on the open page.
fn to(m: &mut Model, t: Instant, item: Item) {
    let view = m.settings_view().unwrap();
    for _ in 0..m.settings_rows(view.page).len() {
        let view = m.settings_view().unwrap();
        if m.settings_rows(view.page)[view.cursor].item == item {
            return;
        }
        m.update(Action::Down, t);
    }
    panic!("{item:?} isn't on {:?}", view.page);
}

fn row(m: &Model, item: Item) -> Row {
    let view = m.settings_view().unwrap();
    m.settings_rows(view.page)
        .into_iter()
        .find(|r| r.item == item)
        .unwrap()
}

#[test]
fn comma_opens_and_closes_and_esc_walks_back() {
    let (mut m, t0) = model_at(temp_config("open"));
    m.update(Action::Settings, t0);
    assert_eq!(m.input_mode(), InputMode::Settings { typing: false });
    m.update(Action::Keep, t0);
    assert!(m.settings_view().unwrap().in_rows);
    m.update(Action::Back, t0);
    assert!(
        !m.settings_view().unwrap().in_rows,
        "esc: back to the pages"
    );
    m.update(Action::Back, t0);
    assert_eq!(m.overlay, Overlay::None, "esc again closes");
    // `,` from help, and `,` again closes.
    m.update(Action::Help, t0);
    m.update(Action::Settings, t0);
    assert!(m.settings_view().is_some());
    m.update(Action::Settings, t0);
    assert_eq!(m.overlay, Overlay::None);
}

#[test]
fn changes_are_live_and_saved() {
    let path = temp_config("live");
    let (mut m, t0) = model_at(path.clone());
    open(&mut m, t0, 0);
    let style = m.style;
    m.update(Action::Change(true), t0);
    assert_ne!(m.style, style, "the lamp changes at once");
    to(&mut m, t0, Item::Heat);
    m.update(Action::Change(false), t0);
    assert_eq!(m.settings.lamp.heat, 2);
    m.update(Action::Close, t0);
    open(&mut m, t0, 1);
    to(&mut m, t0, Item::FocusLength);
    m.update(Action::Change(true), t0);
    assert_eq!(m.settings.pomodoro.focus_min, 30);
    assert_eq!(m.pomodoro.config().focus, Duration::from_secs(30 * 60));
    assert!(row(&m, Item::FocusLength).value.contains("30 min"));
    to(&mut m, t0, Item::Bell);
    m.update(Action::Keep, t0);
    assert!(!m.settings.pomodoro.bell, "enter steps a choice on");
    m.save();
    let (again, _) = model_at(path);
    assert_eq!(again.settings.pomodoro.focus_min, 30);
    assert_eq!(again.settings.lamp.heat, 2);
    assert!(!again.settings.pomodoro.bell);
    assert_eq!(again.style, m.style);
}

#[test]
fn widgets_move_and_their_position_shows_on_the_lamp() {
    let (mut m, t0) = model_at(temp_config("widgets"));
    open(&mut m, t0, 2);
    assert_eq!(m.settings.dock.place(&crate::dock::Clock), Place::Side);
    m.update(Action::Change(true), t0);
    assert_eq!(m.settings.dock.place(&crate::dock::Clock), Place::Overlay);
    assert_eq!(row(&m, Item::Place(0)).value, "on the lamp");
    // On the lamp, it gets a position row.
    m.update(Action::Down, t0);
    assert_eq!(row(&m, Item::Position(0)).value, "centre");
    m.update(Action::Change(true), t0);
    assert_eq!(m.settings.dock.anchor(&crate::dock::Clock), Anchor::Top);
}

#[test]
fn text_on_the_lamp_steps_through_its_modes_and_is_saved() {
    use crate::dock::TextInk;
    let path = temp_config("lamp-text");
    let (mut m, t0) = model_at(path.clone());
    open(&mut m, t0, 2);
    to(&mut m, t0, Item::LampText);
    assert_eq!(row(&m, Item::LampText).value, "automatic");
    m.update(Action::Change(true), t0);
    assert_eq!(m.settings.dock.text, TextInk::Light);
    assert_eq!(row(&m, Item::LampText).value, "light");
    m.update(Action::Change(true), t0);
    assert_eq!(m.settings.dock.text, TextInk::Dark);
    assert_eq!(row(&m, Item::LampText).value, "dark");
    m.update(Action::Change(true), t0);
    assert_eq!(m.settings.dock.text, TextInk::Auto, "and round again");
    m.update(Action::Change(false), t0);
    assert_eq!(m.settings.dock.text, TextInk::Dark);
    m.save();
    let (again, _) = model_at(path);
    assert_eq!(again.settings.dock.text, TextInk::Dark);
}

#[test]
fn reset_needs_a_second_enter_and_puts_the_page_back() {
    let (mut m, t0) = model_at(temp_config("reset"));
    open(&mut m, t0, 1);
    to(&mut m, t0, Item::FocusLength);
    m.update(Action::Change(false), t0);
    assert_eq!(m.settings.pomodoro.focus_min, 20);
    to(&mut m, t0, Item::Reset(Page::Clock));
    m.update(Action::Keep, t0);
    assert_eq!(m.settings.pomodoro.focus_min, 20, "one enter only asks");
    assert_eq!(row(&m, Item::Reset(Page::Clock)).value, "press enter again");
    m.update(Action::Keep, t0);
    assert_eq!(m.settings.pomodoro.focus_min, 25);
    assert_eq!(m.pomodoro.config().focus, Duration::from_secs(25 * 60));
}

#[test]
fn mouse_and_lamp_only_switch_live() {
    let (mut m, t0) = model_at(temp_config("window"));
    open(&mut m, t0, 4);
    m.update(Action::Keep, t0);
    assert!(!m.settings.input.mouse);
    m.update(Action::Close, t0);
    open(&mut m, t0, 5);
    m.update(Action::Keep, t0);
    assert!(m.minimal());
    assert!(m.settings_view().is_some(), "the screen stays open");
    to(&mut m, t0, Item::Smoothness);
    m.update(Action::Change(false), t0);
    assert_eq!(m.settings.display.fps, 45);
}

#[test]
fn client_ids_are_checked() {
    assert_eq!(
        check_client_id(&format!(" {} \n", ID.to_uppercase())),
        Ok(ID.into())
    );
    assert!(check_client_id("").unwrap_err().contains("Paste"));
    assert!(
        check_client_id("abc")
            .unwrap_err()
            .contains("this one has 3")
    );
    assert!(check_client_id("not-an-id").unwrap_err().contains("0-9"));
}

/// Into the Spotify setup.
fn setup(m: &mut Model, t: Instant) {
    open(m, t, 3);
    assert_eq!(row(m, Item::Spotify).value, "not set up");
    m.update(Action::Keep, t);
    assert_eq!(m.settings_view().unwrap().page, Page::Spotify);
}

#[test]
fn the_spotify_setup_walks_through_its_steps() {
    let path = temp_config("spotify");
    let (mut m, t0) = model_at(path.clone());
    let fake = FakeWeb::default();
    let web = fake.clone();
    m.library
        .connect_with(move || Some(Box::new(web.clone()) as Box<dyn Web>));
    setup(&mut m, t0);
    assert!(
        m.media_on(),
        "the player is watched while the setup is open"
    );

    // 2: copy the address (OSC 52, written by the loop).
    to(&mut m, t0, Item::CopyAddress);
    m.update(Action::Keep, t0);
    assert_eq!(m.copy.take().as_deref(), Some(REDIRECT_URI));
    assert_eq!(row(&m, Item::CopyAddress).value, "copied");

    // 4 before 3 says what's missing.
    assert!(row(&m, Item::Connect).about.contains("steps 1 to 3"));

    // 3: type a bad ID, then paste a good one.
    to(&mut m, t0, Item::ClientId);
    m.update(Action::Keep, t0);
    assert_eq!(m.input_mode(), InputMode::Settings { typing: true });
    for c in "q,".chars() {
        m.update(Action::Type(c), t0);
    }
    assert_eq!(
        m.settings_screen.field, "q,",
        "q and , type, they don't close"
    );
    m.update(Action::Keep, t0);
    assert!(matches!(m.settings_screen.note, Some(Err(_))));
    assert!(m.settings.spotify.client_id.is_empty());
    m.paste(&format!("{ID}\n"), t0);
    assert_eq!(m.settings_screen.field, ID);
    m.update(Action::Keep, t0);
    assert_eq!(m.settings.spotify.client_id, ID);
    assert_eq!(m.input_mode(), InputMode::Settings { typing: false });
    let view = m.settings_view().unwrap();
    assert_eq!(
        m.settings_rows(view.page)[view.cursor].item,
        Item::Connect,
        "on to step 4"
    );

    // 4: connect, the browser comes back, connected.
    m.update(Action::Keep, t0);
    assert!(fake.state().login_pending);
    assert_eq!(m.library.account(), Account::LoggingIn);
    assert_eq!(row(&m, Item::Connect).value, "waiting for browser");
    assert!(
        m.settings_rows(Page::Spotify)
            .iter()
            .any(|r| r.item == Item::CopyLoginLink)
    );
    fake.finish_login();
    m.update(Action::Resize, t0);
    assert_eq!(m.library.account(), Account::LoggedIn);
    assert_eq!(row(&m, Item::Connect).value, "connected");
    // Twice to disconnect.
    m.update(Action::Keep, t0);
    assert!(m.library.logged_in());
    m.update(Action::Keep, t0);
    m.update(Action::Resize, t0);
    assert!(!m.library.logged_in());

    // The ID was saved; esc goes back to the music page, on its row.
    m.update(Action::Back, t0);
    let view = m.settings_view().unwrap();
    assert_eq!(view.page, Page::Music);
    assert_eq!(
        m.settings_rows(Page::Music)[view.cursor].item,
        Item::Spotify
    );
    m.update(Action::Close, t0);
    assert!(!m.media_on(), "closed: the player is let go again");
    m.save();
    let (again, _) = model_at(path);
    assert_eq!(again.settings.spotify.client_id, ID);
}

#[test]
fn a_failed_login_says_what_to_check() {
    let (mut m, t0) = model_at(temp_config("spotify-fail"));
    let fake = FakeWeb::default();
    let web = fake.clone();
    m.library
        .connect_with(move || Some(Box::new(web.clone()) as Box<dyn Web>));
    m.settings.spotify.client_id = ID.into();
    open(&mut m, t0, 3);
    m.update(Action::Keep, t0);
    to(&mut m, t0, Item::Connect);
    m.update(Action::Keep, t0);
    fake.fail_login(crate::spotify_web::Error::Login(
        "INVALID_CLIENT: Invalid redirect URI".into(),
    ));
    m.update(Action::Resize, t0);
    let about = row(&m, Item::Connect).about;
    assert!(about.contains("Invalid redirect URI"), "{about}");
    assert!(about.contains("step 2"), "{about}");
}

#[test]
fn pasting_elsewhere_does_nothing() {
    let (mut m, t0) = model_at(temp_config("paste"));
    m.paste(ID, t0);
    assert!(m.settings_screen.field.is_empty());
    open(&mut m, t0, 0);
    m.paste(ID, t0);
    assert!(m.settings_screen.field.is_empty());
    assert!(!m.settings_screen.editing);
}

/// Who can use the Spotify library, and what to do when Spotify says no,
/// read in full in the help lines (lava-1xk.26): nothing is cut at 80×24.
#[test]
fn spotify_eligibility_and_refusal_fit_whole() {
    use super::settings_screen::{ELIGIBILITY, REFUSED};
    use crate::ui::settings::wrap_sentences;
    for text in [ELIGIBILITY, REFUSED] {
        let words = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(words(&wrap_sentences(text, 56, 3).join(" ")), words(text));
    }
    assert!(ELIGIBILITY.contains("Premium") && ELIGIBILITY.contains("At most 5 accounts"));
}

#[test]
fn the_spotify_setup_reads_the_saved_login_and_can_move_it() {
    let (mut m, t0) = model_at(temp_config("spotify-store"));
    let fake = crate::spotify_web::fake::demo().locked();
    let web = fake.clone();
    m.library
        .connect_with(move || Some(Box::new(web.clone()) as Box<dyn Web>));
    m.settings.spotify.client_id = ID.into();
    // The music page says how it stands without reading it.
    m.library.saved = true;
    m.settings.spotify.logged_in = true;
    open(&mut m, t0, 3);
    assert_eq!(fake.state().unlocks, 0);
    // The setup reads it, saying first that macOS may ask.
    m.update(Action::Keep, t0);
    assert_eq!(fake.state().unlocks, 1);
    assert_eq!(
        m.toast.as_ref().map(|t| t.text.as_str()),
        Some(super::library::KEYCHAIN_HEADS_UP)
    );
    m.update(Action::Resize, t0);
    assert!(m.library.logged_in());

    to(&mut m, t0, Item::LoginStore);
    assert_eq!(row(&m, Item::LoginStore).value, "password store");
    m.update(Action::Keep, t0);
    assert_eq!(m.settings.spotify.store, crate::config::LoginStore::File);
    assert_eq!(row(&m, Item::LoginStore).value, "private file");
    assert_eq!(fake.state().moves, [crate::config::LoginStore::File]);
    m.update(Action::Resize, t0);
    assert_eq!(
        m.toast.as_ref().map(|t| t.text.as_str()),
        Some("Spotify login now kept in a private file")
    );
    assert!(m.settings.spotify.logged_in);
}

#[test]
fn saved_lyrics_and_covers_show_their_size_and_clear_on_a_second_enter() {
    use crate::lyrics::cache::tests::TempDir;
    let tmp = TempDir::new("settings-saved");
    let (lyrics, covers) = (tmp.0.join("lyrics"), tmp.0.join("art"));
    std::fs::create_dir_all(&lyrics).unwrap();
    std::fs::create_dir_all(&covers).unwrap();
    std::fs::write(lyrics.join("a.json"), [0; 3000]).unwrap();
    std::fs::write(covers.join("b.img"), [0; 60_000]).unwrap();

    let (mut m, t0) = model_at(temp_config("saved"));
    m.saved_files = SavedFiles::in_folders(lyrics.clone(), covers.clone());
    // Ticks until the worker has answered.
    let settle = |m: &mut Model| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            m.tick(t0, Rect::new(0, 0, 80, 24), local());
            if !m.saved_files.busy() {
                return;
            }
            assert!(Instant::now() < deadline, "no answer");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    open(&mut m, t0, 3);
    to(&mut m, t0, Item::ClearSaved);
    assert_eq!(row(&m, Item::ClearSaved).value, "…");
    settle(&mut m);
    let r = row(&m, Item::ClearSaved);
    assert_eq!(r.value, "63 KB · clear");
    assert!(
        r.about.contains("3 KB of lyrics and 60 KB of covers"),
        "{}",
        r.about
    );

    m.update(Action::Keep, t0);
    assert_eq!(row(&m, Item::ClearSaved).value, "press enter again");
    settle(&mut m);
    assert!(lyrics.join("a.json").exists(), "one enter only asks");
    m.update(Action::Keep, t0);
    settle(&mut m);
    assert!(!lyrics.join("a.json").exists() && !covers.join("b.img").exists());
    assert_eq!(row(&m, Item::ClearSaved).value, "nothing saved");
    assert_eq!(
        m.toast.as_ref().map(|t| t.text.as_str()),
        Some("saved lyrics and covers cleared")
    );
}
