# Подготовка и выпуск релиза

[English](RELEASING.md) | **Русский**

## Перед созданием тега

1. Обновите `version` в `Cargo.toml` и соответствующий пакет в `Cargo.lock`.
2. Добавьте версию и изменения в `CHANGELOG.md`; обновите
   `packaging/RELEASE_NOTES.md` для нового релиза. Обновите также русские версии
   `CHANGELOG.ru.md` и `packaging/RELEASE_NOTES.ru.md`.
3. Выполните проверки и соберите бинарник:

   ```bash
   cargo fmt --check
   cargo test --locked
   cargo clippy --locked --all-targets -- -D warnings
   cargo build --release --locked
   python3 -m unittest discover -s scripts -p 'test_*.py'
   ```

4. Проверьте `--version`, первый запуск, вход, фарм и переключение языка на своём аккаунте.
   Автоматические тесты не подтверждают реальное начисление баллов Twitch.
5. Просмотрите изменения перед коммитом. Конфигурация, сессии и файлы управления
   исключены из Git; не добавляйте их принудительно.

## GitHub Release

После коммита проверенных изменений и отправки ветки `main` создайте тег,
совпадающий с версией пакета. Например, для первого релиза:

```bash
git tag -a v0.1.0 -m "TwFarmer v0.1.0"
git push origin v0.1.0
```

Workflow `Release` запускает проверки на Linux, Windows и macOS, проверяет
совместимость с Rust 1.89 и соответствие версии тегу. Затем собирает архивы:

| Система | Архив |
| --- | --- |
| Linux x86_64, glibc 2.35+ | `twfarmer-vVERSION-x86_64-unknown-linux-gnu.tar.gz` |
| Windows x86_64 | `twfarmer-vVERSION-x86_64-pc-windows-msvc.zip` |
| macOS Apple Silicon | `twfarmer-vVERSION-aarch64-apple-darwin.tar.gz` |

После успешной сборки появляется **черновик** GitHub Release с архивами,
`SHA256SUMS` и описанием на двух языках из `packaging/RELEASE_NOTES.md`
и `packaging/RELEASE_NOTES.ru.md`.
Откройте черновик в GitHub, проверьте вложения и опубликуйте его.
Существующий релиз с таким тегом workflow не перезаписывает.

В настройках репозитория должны быть разрешены GitHub Actions. Право записи
`contents: write` предоставляется только задаче создания черновика; отдельный
персональный токен не требуется. Публикация на crates.io отключена.

## Локальный архив Linux

Нужны Python 3.11+ и собранный бинарник:

```bash
python3 scripts/package_release.py \
  --target x86_64-unknown-linux-gnu \
  --binary target/release/twfarmer
cd dist
sha256sum --check *.sha256
```

Скрипт сверяет версию бинарника и включает только явно перечисленные публичные
файлы. Архив не содержит локальных сессий и конфигурации.
Совместимость локальной Linux-сборки зависит от glibc системы, на которой она
собрана; для распространяемой сборки используйте архив из GitHub Actions.

Для упаковки Windows или macOS передайте соответствующие `--target` и `--binary`
на системе, где этот бинарник может выполняться.
