Vesper for Linux (early test build)

1. Unpack: tar xzf vesper-desktop-linux-x64.tar.gz, then run ./Vesper/vesper-desktop
2. It needs the usual desktop libraries, which most distributions already have: Vulkan (vulkan-icd-loader and a
   Vulkan driver for your GPU), libxkbcommon, Wayland or X11, fontconfig, ALSA (PipeWire and PulseAudio work through
   it), libopus and libpipewire (for sharing your screen). On Arch: pacman -S vulkan-icd-loader libxkbcommon-x11 opus
   pipewire
3. Sign in with the server address (for example https://vesper.tfaucher.com), your username and your password.
   Make the account in the web app first: signup, invites and passkeys are only there.
4. Push to talk works while a game has focus only under X11 (or a game running through XWayland). On a Wayland
   session the push to talk key works while the Vesper window has focus.
5. Staying signed in needs a Secret Service keyring (GNOME Keyring or KWallet). Without one you sign in each time.

The noise filter downloads about 8 MB the first time you join a call. Built on Ubuntu 24.04, so it needs glibc 2.39
or newer. This is a test build. Tell Toby what breaks.
