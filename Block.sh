#!/bin/bash
# Blocks inbound/outbound UDP traffic to selected CS2 server PoPs

API_ENDPOINT="https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730"
BLOCK_FILE="blocked-ips.txt"

# Check dependencies
for dep in curl jq; do
  if ! command -v "$dep" >/dev/null 2>&1; then
    echo "Missing dependency: $dep"
    exit 1
  fi
done

# Detect active firewall
detect_firewall() {
  if command -v ufw >/dev/null && sudo ufw status >/dev/null 2>&1; then
    echo "ufw"
  elif command -v firewall-cmd >/dev/null && sudo firewall-cmd --state >/dev/null 2>&1; then
    echo "firewalld"
  elif command -v nft >/dev/null && sudo nft list ruleset >/dev/null 2>&1; then
    echo "nftables"
  elif command -v iptables >/dev/null && sudo iptables -L >/dev/null 2>&1; then
    echo "iptables"
  else
    echo "none"
  fi
}

# Fetch and validate JSON
fetch_data() {
  local raw
  raw=$(curl -s "$API_ENDPOINT")
  if ! jq empty <<<"$raw" 2>/dev/null; then
    echo "Error: Invalid JSON from Valve API"
    return 1
  fi
  jq 'del(.success, .certs, .p2p_share_ip, .relay_public_key, .revoked_keys, .typical_pings)' <<< "$raw"
}

# List available PoPs
parse_countries() {
  jq -r '.pops | keys[]' <<< "$1"
}

# Get IPs for a PoP
get_ips_by_country() {
  jq -r ".pops[\"$1\"].relays[].ipv4" <<< "$2"
}

# Save IPs to file
save_blocked_ips() {
  local data="$1"; shift
  local countries=("$@")
  > "$BLOCK_FILE"
  for country in "${countries[@]}"; do
    mapfile -t ips < <(get_ips_by_country "$country" "$data")
    printf "%s\n" "${ips[@]}" >> "$BLOCK_FILE"
  done
}

# Apply block rules
block_ip() {
  local ip="$1" fw="$2"
  case "$fw" in
    ufw)
      sudo ufw deny out to "$ip" proto udp
      sudo ufw deny from "$ip" proto udp
      ;;
    firewalld)
      sudo firewall-cmd --permanent --add-rich-rule="rule family='ipv4' source address='$ip' protocol='udp' drop"
      sudo firewall-cmd --permanent --add-rich-rule="rule family='ipv4' destination address='$ip' protocol='udp' drop"
      ;;
    nftables)
      sudo nft add rule inet filter input ip saddr "$ip" udp drop
      sudo nft add rule inet filter output ip daddr "$ip" udp drop
      ;;
    iptables)
      sudo iptables -I INPUT -s "$ip" -p udp -j DROP
      sudo iptables -I OUTPUT -d "$ip" -p udp -j DROP
      ;;
  esac
}

# Remove block rules
unblock_ip() {
  local ip="$1" fw="$2"
  case "$fw" in
    ufw)
      sudo ufw delete deny out to "$ip" proto udp >/dev/null 2>&1
      sudo ufw delete deny from "$ip" proto udp >/dev/null 2>&1
      ;;
    firewalld)
      sudo firewall-cmd --permanent --remove-rich-rule="rule family='ipv4' source address='$ip' protocol='udp' drop"
      sudo firewall-cmd --permanent --remove-rich-rule="rule family='ipv4' destination address='$ip' protocol='udp' drop"
      ;;
    nftables)
      sudo nft delete rule inet filter input ip saddr "$ip" udp drop 2>/dev/null
      sudo nft delete rule inet filter output ip daddr "$ip" udp drop 2>/dev/null
      ;;
    iptables)
      sudo iptables -D INPUT -s "$ip" -p udp -j DROP 2>/dev/null
      sudo iptables -D OUTPUT -d "$ip" -p udp -j DROP 2>/dev/null
      ;;
  esac
}

# Main logic
main() {
  local fw=$(detect_firewall)
  if [[ "$fw" == "none" ]]; then
    echo "No supported firewall detected. Exiting."
    exit 1
  fi

  echo "Detected firewall: $fw"

  if [[ -f "$BLOCK_FILE" ]]; then
    echo "Unblocking previously blocked IPs..."
    while read -r ip; do
      unblock_ip "$ip" "$fw"
    done < "$BLOCK_FILE"
    rm -f "$BLOCK_FILE"
    echo "Unblocked."
  fi

  local data
  if ! data=$(fetch_data); then
    exit 1
  fi

  echo "Available PoPs:"
  parse_countries "$data"

  if [[ $# -gt 0 ]]; then
    save_blocked_ips "$data" "$@"
  else
    read -p "Enter PoP codes to block (space-separated): " -a countries
    save_blocked_ips "$data" "${countries[@]}"
  fi

  echo "Blocking IPs..."
  while read -r ip; do
    block_ip "$ip" "$fw"
  done < "$BLOCK_FILE"
  echo "Done."
}

main "$@"
