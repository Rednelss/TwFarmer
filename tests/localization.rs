use std::{
    path::Path,
    process::{Command, Output},
};

fn run(directory: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_twfarmer"))
        .current_dir(directory)
        .args(arguments)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn english_default_and_russian_help_cover_nested_commands_and_errors() {
    let directory = tempfile::tempdir().unwrap();
    let english = run(directory.path(), &["--help"]);
    assert!(english.status.success());
    assert!(stdout(&english).contains("Commands:"));
    assert!(stdout(&english).contains("Open the management interface"));
    let russian = run(directory.path(), &["--lang", "ru", "--help"]);
    assert!(russian.status.success());
    assert!(stdout(&russian).contains("Команды:"));
    assert!(stdout(&russian).contains("Открыть интерфейс управления"));
    assert!(!stdout(&russian).contains("Print this message"));
    let nested = run(
        directory.path(),
        &["settings", "set", "--lang=ru", "--help"],
    );
    assert!(nested.status.success());
    assert!(stdout(&nested).contains("Сохранённый язык"));
    assert!(stdout(&nested).contains("Забирать Drops"));
    assert!(stdout(&nested).contains("--farm-drops"));
    assert!(stdout(&nested).contains("--auto-drops"));
    assert!(stdout(&nested).contains("--farming-priority"));
    assert!(stdout(&nested).contains("--auto-update"));
    let update = run(directory.path(), &["--lang", "ru", "update", "--help"]);
    assert!(update.status.success());
    assert!(stdout(&update).contains("--check"));
    let drops = run(directory.path(), &["--lang", "ru", "drops", "--help"]);
    assert!(drops.status.success());
    assert!(stdout(&drops).contains("доступные кампании Drops"));
    let login = run(directory.path(), &["--lang", "ru", "login", "--help"]);
    assert!(login.status.success());
    assert!(stdout(&login).contains("--stdin"));
    assert!(!stdout(&login).contains("--token"));
    let invalid = run(directory.path(), &["--lang", "ru", "language", "fr"]);
    assert_eq!(invalid.status.code(), Some(2));
    let error = String::from_utf8_lossy(&invalid.stderr);
    assert!(error.contains("Недопустимое значение"));
    assert!(error.contains("en, ru"));
}

#[test]
fn language_persists_and_flag_overrides_without_rewriting_saved_settings() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        run(directory.path(), &["init", "welovegames"])
            .status
            .success()
    );
    let config = directory.path().join("twfarmer.toml");
    let selected = run(directory.path(), &["language", "ru"]);
    assert!(selected.status.success());
    assert!(stdout(&selected).contains("Язык: Русский"));
    let saved = std::fs::read(&config).unwrap();
    assert!(stdout(&run(directory.path(), &["--help"])).contains("Команды:"));
    assert!(stdout(&run(directory.path(), &["--lang=en", "--help"])).contains("Commands:"));
    assert_eq!(std::fs::read(&config).unwrap(), saved);
    let changed = run(
        directory.path(),
        &[
            "settings",
            "set",
            "--language",
            "en",
            "--chat-presence",
            "false",
            "--farm-drops",
            "false",
            "--claim-drops",
            "false",
            "--auto-drops",
            "true",
            "--farming-priority",
            "drops",
            "--auto-update",
            "false",
        ],
    );
    assert!(changed.status.success());
    assert!(stdout(&changed).contains("Settings saved."));
    let settings: toml::Value = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    assert_eq!(settings["language"].as_str(), Some("en"));
    assert_eq!(settings["chat_presence"].as_bool(), Some(false));
    assert_eq!(settings["farm_drops"].as_bool(), Some(false));
    assert_eq!(settings["claim_drops"].as_bool(), Some(false));
    assert_eq!(settings["auto_drops"].as_bool(), Some(true));
    assert_eq!(settings["farming_priority"].as_str(), Some("drops"));
    assert_eq!(settings["auto_update"].as_bool(), Some(false));
    assert_eq!(settings["channels"][0].as_str(), Some("welovegames"));
}

#[test]
fn daemon_errors_use_selected_language_without_creating_sessions() {
    let directory = tempfile::tempdir().unwrap();
    for (language, expected) in [("en", "Could not read"), ("ru", "Не удалось прочитать")]
    {
        let result = run(
            directory.path(),
            &["--config", "missing.toml", "daemon", "--lang", language],
        );
        assert_eq!(result.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&result.stderr).contains(expected));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
