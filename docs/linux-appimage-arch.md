# Pluely AppImage on Arch Linux

This note records the startup failure observed with `Pluely_0.1.9_amd64.AppImage`
on Arch Linux and the workarounds tested on the affected machine.

## Symptoms and findings

The downloaded AppImage was an executable x86_64 ELF file. Startup exposed two
separate issues:

1. If AppImage cannot access FUSE, it exits with `fuse: device not found` and
   `Cannot mount AppImage`. Arch needs the `fuse2` compatibility package for
   AppImages that use FUSE 2. Check that FUSE is available to the session and
   that `/dev/fuse` exists.
2. With FUSE available, the application could panic with
   `migration 3 was previously applied but is missing in the resolved
   migrations`. The 0.1.9 binary contains migrations 1 and 2, while the
   existing Pluely database had migrations through 7. The database and binary
   therefore came from incompatible application versions.
3. After creating a clean database, the bundled AppImage still produced
   `Could not create default EGL display: EGL_BAD_PARAMETER. Aborting...` and
   showed no application window. The AppImage bundles its own WebKitGTK,
   JavaScriptCoreGTK, and Wayland libraries. On the tested Arch system, running
   the extracted application against Arch's system libraries removed this EGL
   error. This points to a graphics-library compatibility issue in the
   bundled AppImage.

GTK warnings about missing optional modules (`colorreload-gtk-module`,
`window-decorations-gtk-module`, and `appmenu-gtk-module`) were also printed;
they were not the SQLite migration failure.

## Workarounds tested

### Check FUSE

Install the compatibility package if needed:

```sh
sudo pacman -S fuse2
```

The AppImage must also be executable:

```sh
chmod +x Pluely_0.1.9_amd64.AppImage
```

### Recover from the migration mismatch

The database is stored at:

```text
~/.config/com.srikanthnani.pluely/pluely.db
```

Removing it allows this older binary to create a fresh database, but permanently
removes local chat history and settings in that database. Close Pluely first and
back up the database before removing it. SQLite sidecar files, if present, are
`pluely.db-wal` and `pluely.db-shm`.

The affected machine's database was removed after the user explicitly asked to
clear Pluely data; the AppImage then created a fresh database.

### Run with Arch's system WebKitGTK

The following was tested after extracting the AppImage. It starts the bundled
Pluely executable while preferring libraries installed on Arch:

```sh
./Pluely_0.1.9_amd64.AppImage --appimage-extract
APPDIR="$PWD/squashfs-root" \
GDK_BACKEND=x11 \
LD_LIBRARY_PATH=/usr/lib \
"$PWD/squashfs-root/usr/bin/pluely"
```

On the affected machine this eliminated the EGL error, initialized global
shortcuts, and kept the application process running. Automated desktop
inspection could not confirm whether a visible window appeared, so treat this
as a tested rendering workaround rather than a confirmed end-to-end fix.

## Packaging follow-up

The normal AppImage still bundles the older graphics stack and can fail on
Arch. The tested workaround runs the executable outside the AppImage launcher
so it resolves WebKitGTK and related libraries from `/usr/lib`. A durable fix
should be made in the Linux packaging/build configuration and verified on
Arch/Wayland before publishing a replacement AppImage.
