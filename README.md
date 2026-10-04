# CS2 Server Blocker

Native Linux GUI for blocking Counter-Strike 2 Steam SDR relay PoPs by region.

The application fetches the current Steam SDR relay list, lets you select server regions by country, and applies UDP firewall blocks through the detected firewall backend.

## Install

On a supported x86_64 Linux system:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/rekt0ro/CS2-Server-Blocker/main/install.sh)
```

The installer downloads the latest GitHub Release, installs the application to /usr/local/bin/, and adds it to the desktop application menu.

To remove the application:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/rekt0ro/CS2-Server-Blocker/main/uninstall.sh)
```

The uninstaller does not remove your saved application state or firewall rules.

## Features

- Select individual Steam SDR PoPs or whole country groups.
- Block selected regions.
- Unblock selected regions.
- Unblock all regions managed by the application.
- Search the relay list by country, location, or PoP code.
- Scroll through the complete relay list.
- Supports UFW, firewalld, nftables, and iptables.
- Uses pkexec for privileged firewall operations when the application is not already running as root.
- Stores application state under ~/.local/state/cs2-server-blocker/state.json.

## Building from source

Requirements include Rust and the native Linux development libraries used by eframe.

```bash
cargo build --release
cargo run --release
```

The release workflow builds the x86_64 Linux package and publishes it automatically when a version tag such as v0.3.0 is pushed.

## Firewall note

The application only manages rules it creates for CS2 Server Blocker. Existing unrelated firewall rules are left untouched.

Valve can change Steam SDR relay addresses over time, so the application refreshes the relay list from Steam when started and can refresh it again from the GUI.
