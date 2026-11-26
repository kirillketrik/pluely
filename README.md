# Pluely

Pluely is a desktop AI assistant built with Tauri, React, TypeScript, and Rust. It provides a compact overlay and a dashboard for asking questions, working with conversations, and configuring AI and speech providers.

## Features

- **Ask:** chat with configured AI providers, include screenshots, and attach files for context.
- **Listen:** capture microphone and system audio, including selected application streams on Linux through PulseAudio or PipeWire-Pulse; transcribe speech and request AI responses during a conversation.
- **Dashboard:** review chats and listening sessions, manage prompts and files, and configure providers, models, shortcuts, and application settings.
- **Provider options:** use the hosted Pluely models where available, configure compatible services with request templates, or connect supported local AI command-line tools.
- **Local storage:** conversation history and settings are stored on the device using SQLite and browser storage.
- **Desktop integration:** global shortcuts, screen capture, audio device selection, and platform-specific window behavior.

Feature availability depends on the operating system, configured provider, and permissions. Linux audio capture uses PulseAudio-compatible audio services. Consult the [Linux permissions guide](https://docs.pluely.com/docs/getting-started/permissions-linux) for setup details.

## Development requirements

- Node.js and npm
- Rust toolchain with Cargo
- Tauri 2 prerequisites for your operating system

For Linux, install the system packages required by Tauri, including WebKitGTK, GTK, AppIndicator, librsvg, and the PulseAudio development libraries. Package names vary by distribution; see the [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/#linux) for the current list.

## Build on Linux

1. Install the prerequisites above.
2. Install JavaScript dependencies and build the app:

   ```bash
   npm install
   npm run tauri build
   ```

The Tauri configuration requests all supported bundle formats. Linux packages are written under `src-tauri/target/release/bundle/` (commonly `deb/`, `rpm/`, and `appimage/`). To build only one format, pass a bundle target, for example:

```bash
npm run tauri build -- --bundles deb
```

### Arch Linux package

This repository includes a `PKGBUILD` for building a native Arch package without running the AppImage bundler. From an Arch Linux checkout, run:

```bash
makepkg -si
```

The package installs the app binary, desktop entry, and icon. `makepkg` installs the build dependencies declared in the PKGBUILD when prompted. Build dependencies include Node.js, npm, Rust, and Tauri's Linux development libraries; runtime dependencies include WebKitGTK and PulseAudio.

To run the desktop app in development mode:

```bash
npm install
npm run tauri dev
```

The frontend can also be built on its own with `npm run build`.

## Install and permissions

Download the package for your distribution from [pluely.com/download/linux](https://pluely.com/download/linux) or build it using the steps above. On first use, configure the audio sources and grant any permissions required by your desktop environment. Linux screen capture and global shortcuts can vary between Wayland and X11 sessions.

## Documentation and support

- [Documentation](https://docs.pluely.com)
- [Linux permissions](https://docs.pluely.com/docs/getting-started/permissions-linux)
- [Keyboard shortcuts](https://docs.pluely.com/docs/guides/overlay-shortcuts)
- [Report an issue](https://github.com/iamsrikanthnani/pluely/issues)
- [Website and downloads](https://pluely.com)

## License

Pluely v1 and later are proprietary software distributed as binaries. Earlier GPL-3 versions remain available in the repository history.
