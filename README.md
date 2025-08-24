# Linux script for blocking CS2 Server IPs

Block unwanted Counter-Strike 2 servers by region using your Linux firewall — no matter which one you use.  
Supports **UFW**, **iptables**, **nftables**, and **firewalld**.

---

## 🚀 What It Does

- 🛑 Blocks both inbound and outbound UDP traffic to selected CS2 server regions (PoPs)
- 🌍 Pulls live server IPs from Valve’s official API
- 🔥 Works with multiple firewall systems — auto-detects and adapts
- 🧼 Unblocks cleanly when re-run
- 🐧 Designed for Linux gamers who want better matchmaking control

---

## 📦 Requirements

Before running the script, make sure the following tools are installed on your system:

    curl
For fetching server data from Valve.

    jq
For parsing JSON.

# 🐧 Arch
    sudo pacman -S curl jq
# 🐧 Debian
    sudo apt install curl jq
# 🐧 Fedora
    sudo dnf install curl jq
# 🐧 openSUSE
    sudo zypper install curl jq

---

## 🎯 Usage

Block unwanted regions:

Download Block.sh and make it executable:

    chmod +x Block.sh
Now run it like this for example:

    ./Block.sh dxb bom2

Replace dxb bom2 with any PoP codes you want to block.
The script will fetch IPs and apply firewall rules automatically.

Unblock:

    ./Block.sh

When prompted to enter PoPs, press Ctrl+C to exit.
Previously blocked IPs will be removed.

## 🔍 Verify

After blocking, you can check your firewall rules:

UFW:

    sudo ufw status numbered
    
iptables:

    sudo iptables -L -n -v

 nftables:
    
    sudo nft list ruleset

firewalld:

    sudo firewall-cmd --list-rich-rules

## 🌐 What Are PoP Codes?

PoPs (Points of Presence) are Valve’s datacenter identifiers.
Examples:

    dxb → Dubai
    bom2 → Mumbai
    fra → Frankfurt
    ams → Amsterdam

Run the script without arguments to see the full list of available PoPs.
