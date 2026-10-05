# CS2 Server Blocker

A native Linux GUI for blocking Counter-Strike 2 Steam SDR relay regions.

![CS2 Server Blocker screenshot](docs/screenshot.png?v=1.0.1)

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/rekt0ro/CS2-Server-Blocker/main/install.sh | bash
```

### Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/rekt0ro/CS2-Server-Blocker/main/uninstall.sh | bash
```

## Features

- Browse Steam SDR relay PoPs by country
- Search by country, location, or PoP code
- Block selected regions
- Unblock selected regions
- Unblock all managed regions
- Automatic firewall detection
- Supports **UFW**, **firewalld**, **nftables**, and **iptables**
- Persistent local state for blocked regions

## How it works

CS2 Server Blocker fetches the current Steam SDR relay configuration, maps relay PoPs to regions, and applies UDP firewall rules for the regions you select.

## Build from source

```bash
git clone https://github.com/rekt0ro/CS2-Server-Blocker.git
cd CS2-Server-Blocker
cargo run --release
```
