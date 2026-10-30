#!/usr/bin/env bash
# Build and install docex from this checkout: the `docex` command line and
# the `docex-gui` desktop app (Linux and macOS).
#
#   ./install.sh               both, into ~/.local/bin (macOS: the app into ~/Applications)
#   ./install.sh --cli         only the command line
#   ./install.sh --gui         only the desktop app
#   ./install.sh --prefix DIR  binaries into DIR/bin instead
#   ./install.sh --uninstall   remove what this script installed
set -euo pipefail

cd "$(dirname "$0")"
prefix="$HOME/.local"
cli=1
gui=1
uninstall=0
while [ $# -gt 0 ]; do
  case "$1" in
    --cli) gui=0 ;;
    --gui) cli=0 ;;
    --prefix) prefix="$2"; shift ;;
    --uninstall) uninstall=1 ;;
    -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

bin="$prefix/bin"
os="$(uname -s)"
apps="$HOME/.local/share/applications"
icons="$HOME/.local/share/icons/hicolor/128x128/apps"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

if [ "$uninstall" = 1 ]; then
  rm -fv "$bin/docex" "$bin/docex-gui" "$apps/docex.desktop" "$icons/docex.png"
  [ "$os" = Darwin ] && rm -rfv "$HOME/Applications/docex.app"
  exit 0
fi

# ---------------------------------------------------------------------------
# Prerequisites: say exactly what is missing and how to get it.

missing=()
command -v cargo >/dev/null || missing+=("Rust (cargo): https://rustup.rs")
if [ "$gui" = 1 ]; then
  if ! command -v node >/dev/null || ! command -v npm >/dev/null; then
    missing+=("Node.js 20 or newer with npm: https://nodejs.org (or your package manager)")
  elif [ "$(node -p 'process.versions.node.split(".")[0]')" -lt 20 ]; then
    missing+=("Node.js 20 or newer (found $(node --version))")
  fi
  if [ "$os" = Linux ] && ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
    if command -v pacman >/dev/null; then hint="pacman -S --needed base-devel webkit2gtk-4.1 librsvg"
    elif command -v apt-get >/dev/null; then hint="apt-get install build-essential pkg-config libwebkit2gtk-4.1-dev librsvg2-dev"
    elif command -v dnf >/dev/null; then hint="dnf install gcc pkgconf-pkg-config webkit2gtk4.1-devel librsvg2-devel"
    else hint="the WebKitGTK 4.1 development package of your distribution"; fi
    missing+=("WebKitGTK 4.1 (the app's web view), as root: $hint")
  fi
fi
if [ ${#missing[@]} -gt 0 ]; then
  printf 'Missing:\n' >&2
  printf '  - %s\n' "${missing[@]}" >&2
  [ "$gui" = 1 ] && printf '(Only the command line? ./install.sh --cli needs just Rust.)\n' >&2
  exit 1
fi

mkdir -p "$bin"

# ---------------------------------------------------------------------------
# The command line

if [ "$cli" = 1 ]; then
  say "Building the command line"
  cargo build --release --locked -p docex
  install -m 755 target/release/docex "$bin/docex"
  say "Installed $bin/docex"
fi

# ---------------------------------------------------------------------------
# The desktop app: `tauri build` bundles the web front end into the binary
# (a plain `cargo build` of gui/src-tauri doesn't, and can't run on its own).

if [ "$gui" = 1 ]; then
  say "Building the desktop app"
  (cd gui && npm ci --no-audit --no-fund)
  if [ "$os" = Darwin ]; then
    (cd gui && npx tauri build --bundles app)
    mkdir -p "$HOME/Applications"
    rm -rf "$HOME/Applications/docex.app"
    cp -R target/release/bundle/macos/docex.app "$HOME/Applications/"
    ln -sf "$HOME/Applications/docex.app/Contents/MacOS/docex-gui" "$bin/docex-gui"
    say "Installed ~/Applications/docex.app (and $bin/docex-gui)"
  else
    (cd gui && npx tauri build --no-bundle)
    install -m 755 target/release/docex-gui "$bin/docex-gui"
    mkdir -p "$apps" "$icons"
    install -m 644 gui/src-tauri/icons/128x128.png "$icons/docex.png"
    cat > "$apps/docex.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=docex
GenericName=Word document review
Comment=Review and edit Word documents as Markdown, with tracked changes
Exec=$bin/docex-gui %f
Icon=docex
Terminal=false
Categories=Office;WordProcessor;
MimeType=application/vnd.openxmlformats-officedocument.wordprocessingml.document;
EOF
    command -v update-desktop-database >/dev/null && update-desktop-database "$apps" >/dev/null 2>&1 || true
    say "Installed $bin/docex-gui and a menu entry"
  fi
fi

case ":$PATH:" in
  *":$bin:"*) ;;
  *) printf '\nNote: %s is not on your PATH. Add it, e.g. in ~/.bashrc:\n  export PATH="%s:$PATH"\n' "$bin" "$bin" ;;
esac
