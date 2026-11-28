# CAD‑5 · Каталог Elements из Luma

`luma-cad-elements` открывает локальный SQLite-каталог Elements для
чтения и загружает один снимок для всей команды. `elements.find(query)`
возвращает до 16 результатов первой страницы. `elements.select(matches, index)`
берёт результат по индексу с нуля. `elements.resolve(pin)` проверяет
идентификаторы издателя и пакета, версию, хеш манифеста пакета, ID и
дайджест ревизии определения. Описание включает варианты, происхождение
пакета и закреплённые обязательные зависимости. Внешний pin доступен в
команде через `elements.pinned_revision()`.

После установки новой версии каталог продолжает разрешать прежний pin.
Проверка на контрольной пластине сохраняет вариант шириной 40 мм у
старой версии и подтверждает ширину 50 мм при явном выборе нового pin.
Переносимый архив проекта с прежним вариантом и его ведомость
воспроизводятся после установки новой версии.

Профиль проверен на Linux x86‑64, Rust 1.100.0, в режиме `command`.
Адаптер каталога работает в режиме чтения. Для размещения, изменения
конфигурации, подтверждения обновления, ведомости и раскроя используется
автономный CLI Elements с его SQLite workspace и архивами `.lirela`.
Интегрированная запись элементов в Core требует зарегистрированного
состояния и атомарного коммита. [Контракт Elements](https://github.com/maxmurtazin/living-ir-elements/blob/13b06f7ad2885385e27b81d5ee0f118e89217cb0/docs/ADAPTER-CONTRACT.md)

## Воспроизведение

`ELEMENTS` указывает на checkout провайдера из CAD lock. `BUILD` —
новый каталог композиции. Сборка добавляет локальные зависимости Luma
в копию Cargo manifest. Исходный checkout Elements остаётся доступен
для проверки pin и lock. [CAD lock](../../luma-cad.lock.json)

```sh
python3 -B tools/cad/prepare-elements-catalog.py "$ELEMENTS" "$BUILD"
cargo build --manifest-path "$BUILD/Cargo.toml" --bins
ELEMENTS_CLI="$BUILD/target/debug/elements"
CATALOG_CLI="$BUILD/target/debug/luma-cad-elements"
"$ELEMENTS_CLI" catalog-init "$CATALOG"
"$ELEMENTS_CLI" package-build "$ELEMENTS/fixtures/local/plate-package.json" "$PACKAGE"
"$ELEMENTS_CLI" catalog-install "$CATALOG" "$PACKAGE"
"$CATALOG_CLI" "$CATALOG" examples/cad/elements-find-command.json
"$CATALOG_CLI" "$CATALOG" examples/cad/elements-resolve-command.json "$EXACT_PIN"
python3 -B tools/cad/test-elements-catalog.py "$CATALOG_CLI" "$ELEMENTS_CLI" "$ELEMENTS"
```

`EXACT_PIN` — JSON из поля `result.outputs.pin.value.payload` ответа
на первый вызов. Luma отклоняет неправильный дайджест и недоступный pin
без подстановки другой версии пакета.
