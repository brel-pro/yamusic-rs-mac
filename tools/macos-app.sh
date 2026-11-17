#!/bin/bash
# Собирает .app из уже собранного target/release/yamusic-rs: бинарник, Info.plist и значок.
# Подпись своя, без сертификата Apple (ad-hoc). На этом компьютере пакет открывается сразу.
# Скачанный из интернета Gatekeeper остановит, пока пользователь не разрешит запуск в настройках.
# Результат: target/macos/Яндекс Музыка RS.app и zip рядом.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=target/release/yamusic-rs
if [[ ! -x "$BIN" ]]; then
  echo "Сначала соберите релиз: cargo build --release" >&2
  exit 1
fi

VER=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
NAME="Яндекс Музыка RS"
ID=ru.yamusic.rs
OUT=target/macos
APP="$OUT/$NAME.app"
ICONSET="$OUT/AppIcon.iconset"

rm -rf "$APP" "$ICONSET"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$ICONSET"
cp "$BIN" "$APP/Contents/MacOS/yamusic-rs"
chmod 755 "$APP/Contents/MacOS/yamusic-rs"

# Имена, которые ждёт iconutil. Картинки 512 нет, крупнее 256 берём из 256.
cp packaging/icons/16.png  "$ICONSET/icon_16x16.png"
cp packaging/icons/32.png  "$ICONSET/icon_16x16@2x.png"
cp packaging/icons/32.png  "$ICONSET/icon_32x32.png"
cp packaging/icons/64.png  "$ICONSET/icon_32x32@2x.png"
cp packaging/icons/128.png "$ICONSET/icon_128x128.png"
cp packaging/icons/256.png "$ICONSET/icon_128x128@2x.png"
cp packaging/icons/256.png "$ICONSET/icon_256x256.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>${NAME}</string>
  <key>CFBundleDisplayName</key>
  <string>${NAME}</string>
  <key>CFBundleIdentifier</key>
  <string>${ID}</string>
  <key>CFBundleVersion</key>
  <string>${VER}</string>
  <key>CFBundleShortVersionString</key>
  <string>${VER}</string>
  <key>CFBundleExecutable</key>
  <string>yamusic-rs</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.music</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF
plutil -lint "$APP/Contents/Info.plist"

codesign --force --sign - --identifier "$ID" "$APP/Contents/MacOS/yamusic-rs"
codesign --force --sign - "$APP"
codesign --verify --strict "$APP"

case "$(uname -m)" in
  arm64) ARCH=aarch64 ;;
  *) ARCH=$(uname -m) ;;
esac
ZIP="$OUT/yamusic-rs-macos-${ARCH}.zip"
rm -f "$ZIP"
ditto -c -k --keepParent "$APP" "$ZIP"
echo "$ZIP"
