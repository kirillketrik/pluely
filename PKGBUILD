pkgname=pluely
pkgver=0.1.9
pkgrel=1
pkgdesc='Desktop AI assistant with chat and live transcription'
arch=('x86_64')
url='https://github.com/iamsrikanthnani/pluely'
license=('LicenseRef-Proprietary')
depends=('webkit2gtk-4.1' 'gtk3' 'libayatana-appindicator' 'librsvg' 'libpulse')
makedepends=('alsa-lib' 'nodejs' 'npm' 'rust' 'sqlite')
source=('git+https://github.com/kirillketrik/pluely.git#branch=master')
sha256sums=('SKIP')

build() {
  cd "${srcdir}/pluely"
  npm ci
  npm run build
  CARGO_TARGET_DIR="${srcdir}/cargo-target" cargo build --manifest-path src-tauri/Cargo.toml --release --locked
}

package() {
  cd "${srcdir}/pluely"

  install -Dm755 "${srcdir}/cargo-target/release/pluely" "${pkgdir}/usr/bin/pluely"
  install -Dm644 src-tauri/pluely.desktop "${pkgdir}/usr/share/applications/pluely.desktop"
  install -Dm644 src-tauri/icons/128x128.png "${pkgdir}/usr/share/icons/hicolor/128x128/apps/pluely.png"
}
