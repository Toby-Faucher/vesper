Vesper for Linux (early test build)

1. Unpack: tar xzf vesper-desktop-linux-x64.tar.gz, then run ./Vesper/install.sh. It installs Vesper for you alone,
   without root: the app in ~/.local/share/vesper, a link in ~/.local/bin, Vesper in your app menu with its icon, and
   vesper:// links (invites and rooms) opening in Vesper. Run it again from a newer tarball to update by hand, or
   ./Vesper/install.sh --uninstall to remove it. You can also skip it and run ./Vesper/vesper-desktop where it is.
   Or take vesper-desktop-linux-x64.AppImage instead: chmod +x it and run it. It is the same app in one file. It adds
   no menu entry or link handler by itself.
2. It needs the usual desktop libraries, which most distributions already have: Vulkan (vulkan-icd-loader and a
   Vulkan driver for your GPU), libxkbcommon, Wayland or X11, fontconfig, ALSA (PipeWire and PulseAudio work through
   it), libopus and libpipewire (for sharing your screen). On Arch: pacman -S vulkan-icd-loader libxkbcommon-x11 opus
   pipewire
3. Sign in with the server address (for example https://vesper.tfaucher.com), your username and your password.
   New here? Open your invite link and pick "Open in the desktop app", or make the account in the web app.
   Passkeys are only in the web app.
4. Push to talk works while a game has focus only under X11 (or a game running through XWayland). On a Wayland
   session the push to talk key works while the Vesper window has focus.
5. Staying signed in needs a Secret Service keyring (GNOME Keyring or KWallet). Without one you sign in each time.

Updates: Vesper checks GitHub for a new version every few hours and shows "Update and restart" when there is one. It
installs nothing until you click it, and it checks the download's SHA-256 first. It replaces the app file (or the
AppImage) and keeps the old one beside it as .old. Settings > Desktop app turns the check off.

The noise filter downloads about 8 MB the first time you join a call. Built on Ubuntu 24.04, so it needs glibc 2.39
or newer. This is a test build. Tell Toby what breaks.
