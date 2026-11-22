use anyhow::{Context, Result, bail, ensure};
use flate2::read::GzDecoder;
use reqwest::{Client, Url};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    time::Duration,
};

const RELEASE_API: &str = "https://api.github.com/repos/Rednelss/TwFarmer/releases/latest";
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_BINARY: u64 = 128 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub enum Outcome {
    Current,
    Available(String),
    Installed(String),
}

impl Outcome {
    pub fn message(&self) -> String {
        match self {
            Self::Current => tf!(
                "TwFarmer {} is up to date.",
                "TwFarmer {}: обновлений нет.",
                env!("CARGO_PKG_VERSION")
            ),
            Self::Available(version) => tf!(
                "TwFarmer {version} is available.",
                "Доступен TwFarmer {version}."
            ),
            Self::Installed(version) => tf!(
                "TwFarmer {version} installed.",
                "TwFarmer {version} установлен."
            ),
        }
    }
}

pub fn target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") if cfg!(target_env = "gnu") => Ok("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") if cfg!(target_env = "msvc") => Ok("x86_64-pc-windows-msvc"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        _ => bail!(
            "{}",
            tf!(
                "No GitHub release build for this platform",
                "Для этой платформы нет сборки в GitHub Releases"
            )
        ),
    }
}

fn client() -> Result<Client> {
    Client::builder()
        .user_agent(concat!("TwFarmer/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            let trusted = url.host_str().is_some_and(|host| {
                host == "github.com"
                    || host == "api.github.com"
                    || host == "githubusercontent.com"
                    || host.ends_with(".githubusercontent.com")
            });
            if attempt.previous().len() > 5
                || url.scheme() != "https"
                || !trusted
                || url.port().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                attempt.error("Invalid GitHub download redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .context(tr!(
            "Could not create the updater HTTP client",
            "Не удалось создать HTTP-клиент обновлений"
        ))
}

fn release_version(release: &Release, current: &str) -> Result<Option<Version>> {
    ensure!(
        !release.draft && !release.prerelease,
        "{}",
        tr!(
            "Expected a published stable release",
            "Нужен опубликованный стабильный релиз"
        )
    );
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .context(tr!(
        "Invalid GitHub release version",
        "Неверная версия релиза GitHub"
    ))?;
    ensure!(
        version.pre.is_empty(),
        "{}",
        tr!(
            "Prerelease updates are not supported",
            "Обновления на предварительные версии не поддерживаются"
        )
    );
    let current = Version::parse(current)?;
    Ok((version > current).then_some(version))
}

fn asset_url(release: &Release, name: &str) -> Result<Url> {
    let mut matches = release.assets.iter().filter(|asset| asset.name == name);
    let asset = matches.next().with_context(|| {
        tf!(
            "Release asset missing: {name}",
            "В релизе нет файла: {name}"
        )
    })?;
    ensure!(
        matches.next().is_none(),
        "{}",
        tr!("Duplicate release asset", "Повторяющийся файл релиза")
    );
    let url = Url::parse(&asset.browser_download_url)?;
    let expected = format!(
        "/Rednelss/TwFarmer/releases/download/{}/{name}",
        release.tag_name
    );
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("github.com")
            && url.path() == expected
            && url.port().is_none()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "{}",
        tr!(
            "Invalid GitHub release asset URL",
            "Неверный адрес файла релиза GitHub"
        )
    );
    Ok(url)
}

async fn download(client: &Client, url: Url, file: &mut fs::File, limit: u64) -> Result<()> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.without_url())
        .context(tr!(
            "Downloading a GitHub release file",
            "Скачивание файла релиза GitHub"
        ))?;
    crate::twitch::check_status(
        response.status(),
        tr!("GitHub download", "Скачивание с GitHub"),
    )?;
    ensure!(
        response.content_length().is_none_or(|size| size <= limit),
        "{}",
        tr!("Release file is too large", "Файл релиза слишком большой")
    );
    let mut size = 0;
    while let Some(bytes) = response.chunk().await.map_err(|e| e.without_url())? {
        size += bytes.len() as u64;
        ensure!(
            size <= limit,
            "{}",
            tr!("Release file is too large", "Файл релиза слишком большой")
        );
        file.write_all(&bytes)?;
    }
    file.sync_all()?;
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

fn expected_checksum(text: &str, name: &str) -> Result<String> {
    let mut found = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Some((digest, file)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if file.trim().trim_start_matches('*') != name {
            continue;
        }
        ensure!(
            found.is_none() && digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "{}",
            tr!(
                "Invalid or duplicate release checksum",
                "Неверная или повторяющаяся контрольная сумма релиза"
            )
        );
        found = Some(digest.to_ascii_lowercase());
    }
    found.context(tr!(
        "Archive checksum missing from SHA256SUMS",
        "В SHA256SUMS нет контрольной суммы архива"
    ))
}

fn verify(file: &mut fs::File, expected: &str) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    ensure!(
        format!("{:x}", hash.finalize()) == expected,
        "{}",
        tr!(
            "Release checksum does not match. The executable was not changed.",
            "Контрольная сумма релиза не совпала. Исполняемый файл не изменён."
        )
    );
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

fn write_binary(reader: impl Read, size: u64, output: &mut fs::File) -> Result<()> {
    ensure!(
        size > 0 && size <= MAX_BINARY,
        "{}",
        tr!(
            "Invalid release executable size",
            "Неверный размер исполняемого файла релиза"
        )
    );
    let copied = std::io::copy(&mut reader.take(MAX_BINARY + 1), output)?;
    ensure!(
        copied == size,
        "{}",
        tr!(
            "Incomplete release executable",
            "Неполный исполняемый файл релиза"
        )
    );
    output.sync_all()?;
    Ok(())
}

fn extract(archive: &mut fs::File, output: &mut fs::File, root: &str, windows: bool) -> Result<()> {
    let expected = format!(
        "{root}/{}",
        if windows { "twfarmer.exe" } else { "twfarmer" }
    );
    if windows {
        let mut archive = zip::ZipArchive::new(archive)?;
        ensure!(
            archive
                .file_names()
                .filter(|name| *name == expected)
                .count()
                == 1,
            "{}",
            tr!(
                "Release archive must contain one executable",
                "В архиве релиза должен быть один исполняемый файл"
            )
        );
        let entry = archive.by_name(&expected)?;
        ensure!(
            !entry.is_dir()
                && entry
                    .unix_mode()
                    .is_none_or(|mode| mode & 0o170000 == 0 || mode & 0o170000 == 0o100000),
            "{}",
            tr!(
                "Invalid executable entry in release archive",
                "Неверная запись исполняемого файла в архиве"
            )
        );
        let size = entry.size();
        write_binary(entry, size, output)?;
    } else {
        let mut archive = tar::Archive::new(GzDecoder::new(archive).take(MAX_BINARY + MAX_ARCHIVE));
        let mut found = false;
        for entry in archive.entries()? {
            let entry = entry?;
            if entry.path()?.as_ref() != Path::new(&expected) {
                continue;
            }
            ensure!(
                !found && entry.header().entry_type().is_file(),
                "{}",
                tr!(
                    "Invalid or duplicate executable entry",
                    "Неверная или повторяющаяся запись исполняемого файла"
                )
            );
            let size = entry.size();
            write_binary(entry, size, output)?;
            found = true;
        }
        ensure!(
            found,
            "{}",
            tr!(
                "Executable missing from release archive",
                "В архиве релиза нет исполняемого файла"
            )
        );
    }
    Ok(())
}

pub async fn run(check_only: bool) -> Result<Outcome> {
    let target = target()?;
    let client = client()?;
    let response = client
        .get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.without_url())
        .context(tr!("Checking GitHub releases", "Проверка релизов GitHub"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Outcome::Current);
    }
    crate::twitch::check_status(response.status(), "GitHub Releases")?;
    let release: Release = response.json().await.context(tr!(
        "Invalid GitHub release response",
        "Неверный ответ релизов GitHub"
    ))?;
    let Some(version) = release_version(&release, env!("CARGO_PKG_VERSION"))? else {
        return Ok(Outcome::Current);
    };
    let root = format!("twfarmer-v{version}-{target}");
    let windows = target.contains("windows");
    let name = format!("{root}.{}", if windows { "zip" } else { "tar.gz" });
    let archive_url = asset_url(&release, &name)?;
    let checksum_url = asset_url(&release, "SHA256SUMS")?;
    if check_only {
        return Ok(Outcome::Available(version.to_string()));
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let runtime = std::process::Command::new("getconf")
            .arg("GNU_LIBC_VERSION")
            .output()
            .context(tr!(
                "Could not determine glibc compatibility",
                "Не удалось проверить совместимость glibc"
            ))?;
        let value = String::from_utf8_lossy(&runtime.stdout);
        let parts: Vec<_> = value
            .trim()
            .strip_prefix("glibc ")
            .unwrap_or("")
            .split('.')
            .filter_map(|part| part.parse::<u32>().ok())
            .collect();
        ensure!(
            runtime.status.success() && parts.len() >= 2 && (parts[0], parts[1]) >= (2, 35),
            "{}",
            tr!(
                "GitHub Linux builds require glibc 2.35 or newer. Build from source on this system.",
                "Сборки GitHub для Linux требуют glibc 2.35 или новее. На этой системе соберите программу из исходников."
            )
        );
    }
    let mut sums = tempfile::tempfile()?;
    download(&client, checksum_url, &mut sums, 1024 * 1024).await?;
    let mut checksum_text = String::new();
    sums.read_to_string(&mut checksum_text)?;
    let expected = expected_checksum(&checksum_text, &name)?;
    let mut archive = tempfile::tempfile()?;
    download(&client, archive_url, &mut archive, MAX_ARCHIVE).await?;
    verify(&mut archive, &expected)?;
    let executable = std::env::current_exe()?;
    let directory = executable.parent().context(tr!(
        "Executable directory missing",
        "Нет каталога исполняемого файла"
    ))?;
    let mut replacement = tempfile::NamedTempFile::new_in(directory).context(tr!(
        "Executable directory is not writable; install the update manually.",
        "Нет прав записи в каталог программы; установите обновление вручную."
    ))?;
    extract(&mut archive, replacement.as_file_mut(), &root, windows)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&executable)?.permissions().mode() & 0o777;
        replacement
            .as_file()
            .set_permissions(fs::Permissions::from_mode(mode))?;
    }
    self_replace::self_replace(replacement.path())
        .context(tr!("Replacing the executable", "Замена исполняемого файла"))?;
    Ok(Outcome::Installed(version.to_string()))
}

pub fn restart(executable: &Path, args: &[std::ffi::OsString]) -> Result<()> {
    let mut command = std::process::Command::new(executable);
    command.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec()).context(tr!("Restarting TwFarmer", "Перезапуск TwFarmer"))
    }
    #[cfg(not(unix))]
    {
        command
            .spawn()
            .context(tr!("Restarting TwFarmer", "Перезапуск TwFarmer"))?;
        std::process::exit(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use zip::write::SimpleFileOptions;

    #[test]
    fn replaces_and_restarts_a_temporary_executable_without_changing_private_files() {
        let directory = tempfile::tempdir().unwrap();
        let suffix = std::env::consts::EXE_SUFFIX;
        let executable = directory.path().join(format!("twfarmer-test{suffix}"));
        let next = directory.path().join(format!("next{suffix}"));
        fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        fs::copy(&executable, &next).unwrap();
        fs::write(
            directory.path().join("twfarmer.toml"),
            b"settings unchanged",
        )
        .unwrap();
        fs::write(
            directory.path().join("session.auth.json"),
            b"session unchanged",
        )
        .unwrap();
        let output = std::process::Command::new(&executable)
            .args([
                "--exact",
                "update::tests::replacement_fixture",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(
                "TWFARMER_TEST_UPDATE_DIRECTORY",
                directory.path().canonicalize().unwrap(),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let marker = directory.path().join("restarted");
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "Updated process did not restart"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            fs::read(directory.path().join("twfarmer.toml")).unwrap(),
            b"settings unchanged"
        );
        assert_eq!(
            fs::read(directory.path().join("session.auth.json")).unwrap(),
            b"session unchanged"
        );
    }

    // These fixtures run only in a copy of the test binary created by the test above.
    fn assert_fixture_directory(executable: &Path, root: &Path) {
        assert_eq!(
            executable.parent().unwrap().canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn fixture_directory_accepts_aliases_of_the_same_directory() {
        let directory = tempfile::tempdir().unwrap();
        let actual = directory.path().join("actual");
        let alias = directory.path().join("alias");
        fs::create_dir(&actual).unwrap();
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        assert_fixture_directory(&alias.join("twfarmer"), &actual);
        assert_fixture_directory(&actual.join("twfarmer"), &alias);
    }

    #[test]
    fn replacement_fixture() {
        let Some(root) = std::env::var_os("TWFARMER_TEST_UPDATE_DIRECTORY") else {
            return;
        };
        let executable = std::env::current_exe().unwrap();
        assert_fixture_directory(&executable, Path::new(&root));
        fs::write(
            Path::new(&root).join("before.pid"),
            std::process::id().to_string(),
        )
        .unwrap();
        self_replace::self_replace(
            Path::new(&root).join(format!("next{}", std::env::consts::EXE_SUFFIX)),
        )
        .unwrap();
        restart(
            &executable,
            &[
                "--exact".into(),
                "update::tests::restart_fixture".into(),
                "--test-threads=1".into(),
            ],
        )
        .unwrap();
    }

    #[test]
    fn restart_fixture() {
        let Some(root) = std::env::var_os("TWFARMER_TEST_UPDATE_DIRECTORY") else {
            return;
        };
        let root = Path::new(&root);
        assert_fixture_directory(&std::env::current_exe().unwrap(), root);
        #[cfg(unix)]
        {
            let previous_pid: u32 = fs::read_to_string(root.join("before.pid"))
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(previous_pid, std::process::id());
        }
        assert_eq!(
            fs::read(root.join("session.auth.json")).unwrap(),
            b"session unchanged"
        );
        fs::write(root.join("restarted"), b"ok").unwrap();
    }

    fn release(version: &str) -> Release {
        Release {
            tag_name: version.into(),
            draft: false,
            prerelease: false,
            assets: vec![],
        }
    }

    #[test]
    fn stable_releases_do_not_downgrade_or_install_prereleases() {
        assert!(
            release_version(&release("v0.3.1"), "0.3.0")
                .unwrap()
                .is_some()
        );
        for version in ["v0.3.0", "v0.2.9"] {
            assert!(
                release_version(&release(version), "0.3.0")
                    .unwrap()
                    .is_none()
            );
        }
        let mut upcoming = release("v0.4.0");
        upcoming.draft = true;
        assert!(release_version(&upcoming, "0.3.0").is_err());
        upcoming.draft = false;
        upcoming.prerelease = true;
        assert!(release_version(&upcoming, "0.3.0").is_err());
        assert!(release_version(&release("v0.4.0-rc.1"), "0.3.0").is_err());
        assert!(release_version(&release("latest"), "0.3.0").is_err());
    }

    #[test]
    fn assets_must_belong_to_the_exact_release_and_repository() {
        let mut release = release("v0.3.1");
        assert!(asset_url(&release, "SHA256SUMS").is_err());
        let valid = "https://github.com/Rednelss/TwFarmer/releases/download/v0.3.1/SHA256SUMS";
        release.assets.push(Asset {
            name: "SHA256SUMS".into(),
            browser_download_url: valid.into(),
        });
        assert_eq!(asset_url(&release, "SHA256SUMS").unwrap().as_str(), valid);
        for invalid in [
            valid.replace("https://", "http://"),
            valid.replace("github.com", "other.test"),
            valid.replace("Rednelss", "other"),
            valid.replace("v0.3.1", "v0.3.2"),
            format!("{valid}?redirect=other"),
        ] {
            release.assets[0].browser_download_url = invalid;
            assert!(asset_url(&release, "SHA256SUMS").is_err());
        }
    }

    #[test]
    fn checksums_select_the_exact_filename_and_detect_corruption() {
        let bytes = b"release archive bytes";
        let digest = format!("{:x}", Sha256::digest(bytes));
        let sums = format!(
            "{}  other.zip\n{}  release.tar.gz\n",
            "0".repeat(64),
            digest.to_uppercase()
        );
        assert_eq!(expected_checksum(&sums, "release.tar.gz").unwrap(), digest);
        assert!(expected_checksum(&sums, "missing.zip").is_err());
        assert!(expected_checksum(&(sums.clone() + &sums), "release.tar.gz").is_err());
        assert!(expected_checksum("wrong  release.tar.gz", "release.tar.gz").is_err());
        let mut archive = tempfile::tempfile().unwrap();
        archive.write_all(bytes).unwrap();
        verify(&mut archive, &digest).unwrap();
        archive.write_all(b"corrupted").unwrap();
        assert!(verify(&mut archive, &digest).is_err());
    }

    fn tar_archive(entries: &[(&str, &[u8])]) -> fs::File {
        let mut builder = tar::Builder::new(GzEncoder::new(vec![], Compression::fast()));
        for (name, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, name, *bytes).unwrap();
        }
        let bytes = builder.into_inner().unwrap().finish().unwrap();
        let mut archive = tempfile::tempfile().unwrap();
        archive.write_all(&bytes).unwrap();
        archive.seek(SeekFrom::Start(0)).unwrap();
        archive
    }

    #[test]
    fn tar_extracts_only_the_exact_binary_and_rejects_duplicates_and_symlinks() {
        let mut archive = tar_archive(&[
            ("release/twfarmer", b"binary"),
            ("release/twfarmer.toml", b"untrusted settings"),
            ("other/twfarmer", b"wrong binary"),
        ]);
        let mut output = tempfile::tempfile().unwrap();
        extract(&mut archive, &mut output, "release", false).unwrap();
        output.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = vec![];
        output.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"binary");
        let mut duplicate = tar_archive(&[
            ("release/twfarmer", b"first"),
            ("release/twfarmer", b"second"),
        ]);
        assert!(
            extract(
                &mut duplicate,
                &mut tempfile::tempfile().unwrap(),
                "release",
                false
            )
            .is_err()
        );
        let mut wrong_root = tar_archive(&[("other/twfarmer", b"wrong")]);
        assert!(
            extract(
                &mut wrong_root,
                &mut tempfile::tempfile().unwrap(),
                "release",
                false
            )
            .is_err()
        );
        let mut builder = tar::Builder::new(GzEncoder::new(vec![], Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("../private-file").unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, "release/twfarmer", &[][..])
            .unwrap();
        let bytes = builder.into_inner().unwrap().finish().unwrap();
        let mut archive = tempfile::tempfile().unwrap();
        archive.write_all(&bytes).unwrap();
        archive.seek(SeekFrom::Start(0)).unwrap();
        assert!(
            extract(
                &mut archive,
                &mut tempfile::tempfile().unwrap(),
                "release",
                false
            )
            .is_err()
        );
    }

    #[test]
    fn windows_zip_extracts_the_binary_without_overwriting_private_files() {
        let mut archive = tempfile::tempfile().unwrap();
        {
            let mut writer = zip::ZipWriter::new(&mut archive);
            for (name, bytes) in [
                ("release/twfarmer.exe", &b"windows binary"[..]),
                ("release/twfarmer.toml.auth.json", &b"private"[..]),
            ] {
                writer
                    .start_file(
                        name,
                        SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated),
                    )
                    .unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        archive.seek(SeekFrom::Start(0)).unwrap();
        let mut output = tempfile::tempfile().unwrap();
        extract(&mut archive, &mut output, "release", true).unwrap();
        output.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = vec![];
        output.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"windows binary");
    }
}
