use crate::config::Multiplexer;
use crate::proc::fake;
use crate::session::{Mux, MuxEnv, SetupError, detect_mux, hc, herdr, tmux};

#[test]
fn a_missing_multiplexer_is_named_in_the_setup_error() {
    assert_eq!(SetupError::ProgramMissing("herdr").to_string(), "herdr is not installed or not on PATH");
    assert!(herdr::Place::detect(Some("1"), None, "orbit").is_err());
}

fn env(herdr: bool, tmux: bool) -> MuxEnv {
    MuxEnv {
        herdr_env: herdr.then(|| "1".to_string()),
        herdr_workspace: herdr.then(|| "w1".to_string()),
        tmux: tmux.then(|| "/tmp/tmux-1000/default,1,0".to_string()),
        hc_socket: None,
    }
}

fn in_app(env: MuxEnv) -> MuxEnv {
    MuxEnv { hc_socket: Some("/run/user/1000/herdr-core/default.sock".to_string()), ..env }
}

#[test]
fn the_multiplexer_choice_picks_the_backend_from_the_environment() {
    use Multiplexer::{Auto, Herdr, Tmux};
    let inside_herdr = Mux::Herdr(herdr::Place::detect(Some("1"), Some("w1"), "orbit").expect("inside"));
    let detached_herdr = Mux::Herdr(herdr::Place::detect(None, None, "orbit").expect("detached"));
    let inside_tmux = Mux::Tmux(tmux::Place::Inside { session_id: "$1".to_string() });
    let detached_tmux = Mux::Tmux(tmux::Place::Detached { session_name: "dispatch-orbit".to_string() });
    let table = [
        (Auto, false, false, &detached_tmux),
        (Auto, false, true, &inside_tmux),
        (Auto, true, false, &inside_herdr),
        (Auto, true, true, &inside_herdr),
        (Tmux, false, false, &detached_tmux),
        (Tmux, false, true, &inside_tmux),
        (Tmux, true, false, &detached_tmux),
        (Tmux, true, true, &inside_tmux),
        (Herdr, false, false, &detached_herdr),
        (Herdr, false, true, &detached_herdr),
        (Herdr, true, false, &inside_herdr),
        (Herdr, true, true, &inside_herdr),
    ];
    for (choice, in_herdr, in_tmux, want) in table {
        fake::install(|call| {
            Ok(if call.starts_with("tmux display-message") { "$1\n".to_string() } else { String::new() })
        });
        let got = detect_mux(choice, &env(in_herdr, in_tmux), std::path::Path::new("/w/orbit")).expect("detects");
        assert_eq!(&got, want, "{choice:?} herdr={in_herdr} tmux={in_tmux}");
    }
}

#[test]
fn the_herdr_app_is_chosen_by_setting_or_by_its_socket() {
    use Multiplexer::{Auto, Herdr, HerdrApp, Tmux};
    let tmux_detached = Mux::Tmux(tmux::Place::Detached { session_name: "dispatch-orbit".to_string() });
    let herdr_inside = Mux::Herdr(herdr::Place::detect(Some("1"), Some("w1"), "orbit").expect("inside"));
    let (app, outside) = (Mux::Hc(hc::Place::App), Mux::Hc(hc::Place::Outside));
    let table = [
        (Auto, in_app(env(false, false)), &app),
        (Auto, in_app(env(false, true)), &app),
        (Auto, in_app(env(true, false)), &herdr_inside),
        (Auto, env(false, false), &tmux_detached),
        (Tmux, in_app(env(false, false)), &tmux_detached),
        (Herdr, in_app(env(true, false)), &herdr_inside),
        (HerdrApp, env(false, false), &outside),
        (HerdrApp, env(true, true), &outside),
        (HerdrApp, in_app(env(false, false)), &app),
        (HerdrApp, in_app(env(true, true)), &app),
    ];
    for (choice, e, want) in table {
        fake::install(|_| Ok(String::new()));
        let got = detect_mux(choice, &e, std::path::Path::new("/w/orbit")).expect("detects");
        assert_eq!(&got, want, "{choice:?} {e:?}");
    }
    fake::install(|call| {
        if call.starts_with("hc") {
            Err(crate::proc::ProcError::Spawn {
                program: "hc".to_string(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            })
        } else {
            Ok(String::new())
        }
    });
    assert!(matches!(
        detect_mux(HerdrApp, &env(false, false), std::path::Path::new("/w/orbit")),
        Err(SetupError::ProgramMissing("hc"))
    ));
}

#[test]
fn the_chosen_multiplexer_must_be_installed() {
    fake::install(|_| Err(fake::fail("x", 1, "")));
    let spawn = || crate::proc::ProcError::Spawn {
        program: "x".to_string(),
        source: std::io::Error::from(std::io::ErrorKind::NotFound),
    };
    fake::install(move |call| if call.starts_with("herdr") { Err(spawn()) } else { Ok(String::new()) });
    let err = detect_mux(Multiplexer::Herdr, &env(false, true), std::path::Path::new("/w/orbit"));
    assert!(matches!(err, Err(SetupError::ProgramMissing("herdr"))));
    assert!(detect_mux(Multiplexer::Tmux, &env(false, false), std::path::Path::new("/w/orbit")).is_ok());
    assert!(matches!(
        detect_mux(Multiplexer::Auto, &env(true, false), std::path::Path::new("/w/orbit")),
        Err(SetupError::ProgramMissing("herdr"))
    ));
}
