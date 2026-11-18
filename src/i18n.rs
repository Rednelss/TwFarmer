use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fmt,
    path::PathBuf,
    sync::atomic::{AtomicU8, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Ru,
}

impl Language {
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Ru => "ru",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Ru => "Русский",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::str::FromStr for Language {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "en" => Ok(Self::En),
            "ru" => Ok(Self::Ru),
            _ => Err(text(
                "Language must be en or ru",
                "Язык должен быть en или ru",
            )),
        }
    }
}

static LANGUAGE: AtomicU8 = AtomicU8::new(0);
static OVERRIDE: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Language {
    if LANGUAGE.load(Ordering::Relaxed) == 1 {
        Language::Ru
    } else {
        Language::En
    }
}

pub fn set_override(language: Option<Language>) {
    OVERRIDE.store(
        language.map_or(0, |lang| if lang == Language::Ru { 2 } else { 1 }),
        Ordering::Relaxed,
    );
    if let Some(language) = language {
        set(language);
    }
}

fn set(language: Language) {
    LANGUAGE.store(u8::from(language == Language::Ru), Ordering::Relaxed);
}

pub fn apply_config(language: Language) {
    set(match OVERRIDE.load(Ordering::Relaxed) {
        1 => Language::En,
        2 => Language::Ru,
        _ => language,
    });
}

pub fn text(english: &'static str, russian: &'static str) -> &'static str {
    select(current(), english, russian)
}

fn select(language: Language, english: &'static str, russian: &'static str) -> &'static str {
    if language == Language::Ru {
        russian
    } else {
        english
    }
}

// Read the language early so clap can translate help and errors.
pub fn startup_language(arguments: &[OsString]) -> (Language, Option<Language>) {
    let mut config = PathBuf::from("twfarmer.toml");
    let mut explicit = None;
    let mut args = arguments.iter().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--" {
            break;
        }
        if argument == "--config" {
            if let Some(value) = args.next() {
                config = PathBuf::from(value);
            }
        } else if argument == "--lang" {
            explicit = args
                .next()
                .and_then(|value| value.to_str())
                .and_then(|value| value.parse().ok());
        } else if let Some(value) = argument
            .to_str()
            .and_then(|value| value.strip_prefix("--config="))
        {
            config = PathBuf::from(value);
        } else if let Some(value) = argument
            .to_str()
            .and_then(|value| value.strip_prefix("--lang="))
        {
            explicit = value.parse().ok();
        }
    }
    let saved = std::fs::read_to_string(config)
        .ok()
        .and_then(|source| source.parse::<toml::Value>().ok())
        .and_then(|value| {
            value
                .get("language")
                .and_then(toml::Value::as_str)
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or_default();
    (explicit.unwrap_or(saved), explicit)
}

pub fn initialize(arguments: &[OsString]) {
    let (language, explicit) = startup_language(arguments);
    set_override(explicit);
    set(language);
}

macro_rules! tr {
    ($english:literal, $russian:literal) => {
        $crate::i18n::text($english, $russian)
    };
}

macro_rules! tf {
    ($english:literal, $russian:literal $(, $args:expr)* $(,)?) => {
        if $crate::i18n::current() == $crate::i18n::Language::Ru {
            format!($russian $(, $args)*)
        } else {
            format!($english $(, $args)*)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_have_stable_codes_and_native_names() {
        assert_eq!("en".parse::<Language>().unwrap(), Language::En);
        assert_eq!("ru".parse::<Language>().unwrap(), Language::Ru);
        assert!("fr".parse::<Language>().is_err());
        assert_eq!(select(Language::En, "Help", "Помощь"), "Help");
        assert_eq!(select(Language::Ru, "Help", "Помощь"), "Помощь");
        assert_eq!(Language::En.name(), "English");
        assert_eq!(Language::Ru.name(), "Русский");
    }

    #[test]
    fn startup_respects_config_and_explicit_language_in_any_position() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("settings.toml");
        std::fs::write(&config, "language = 'ru'\n").unwrap();
        let arguments = vec![
            OsString::from("twfarmer"),
            OsString::from("--config"),
            config.clone().into_os_string(),
            OsString::from("--help"),
        ];
        assert_eq!(startup_language(&arguments), (Language::Ru, None));
        let mut explicit = arguments.clone();
        explicit.push(OsString::from("--lang=en"));
        assert_eq!(
            startup_language(&explicit),
            (Language::En, Some(Language::En))
        );
        let args = vec![
            OsString::from("twfarmer"),
            OsString::from("--config"),
            config.into_os_string(),
            OsString::from("--"),
            OsString::from("--lang=en"),
        ];
        assert_eq!(startup_language(&args), (Language::Ru, None));
    }
}
