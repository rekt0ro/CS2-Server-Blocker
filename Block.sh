#!/bin/bash

API_ENDPOINT="https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730"
BLOCK_FILE="blocked-ips.txt"
TMP_DATA="/tmp/cs2_data.json"

# 🎨 Terminal colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# 🌀 Spinner
spinner() {
  local pid=$!
  local delay=0.1
  local spinstr='|/-\'
  while ps -p $pid >/dev/null 2>&1; do
    local temp=${spinstr#?}
    printf " [%c]  " "$spinstr"
    spinstr=$temp${spinstr%"$temp"}
    sleep $delay
    printf "\b\b\b\b\b\b"
  done
}

# 🖼️ Banner
print_banner() {
  echo
  if command -v figlet >/dev/null; then
    figlet -c "CS2 PoP Blocker"
  elif command -v toilet >/dev/null; then
    toilet -f mono12 -F metal "CS2 PoP Blocker"
  else
    echo -e "${BOLD}${BLUE}=== CS2 PoP Blocker ===${NC}"
  fi
  echo
}

# 🔍 Check dependencies
check_dependencies() {
  for dep in curl jq column; do
    if ! command -v "$dep" >/dev/null 2>&1; then
      echo -e "${RED}❌ Missing dependency: ${dep}${NC}"
      exit 1
    fi
  done
}

# 🔥 Detect active firewall
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

# 🌐 Fetch and validate JSON
fetch_data() {
  curl -s "$API_ENDPOINT" > "$TMP_DATA"
  if ! jq empty < "$TMP_DATA" 2>/dev/null; then
    echo -e "${RED}❌ Error: Invalid JSON from Valve API${NC}"
    return 1
  fi
  jq 'del(.success, .certs, .p2p_share_ip, .relay_public_key, .revoked_keys, .typical_pings)' "$TMP_DATA" > "$TMP_DATA.cleaned"
  mv "$TMP_DATA.cleaned" "$TMP_DATA"
}

# 📍 List available PoPs
parse_countries() {
  jq -r '.pops | keys[]' < "$TMP_DATA"
}

# 📡 Get IPs for a PoP
get_ips_by_country() {
  jq -r ".pops[\"$1\"].relays[].ipv4" < "$TMP_DATA"
}

# 💾 Save IPs to file
save_blocked_ips() {
  local countries=("$@")
  > "$BLOCK_FILE"
  for country in "${countries[@]}"; do
    mapfile -t ips < <(get_ips_by_country "$country")
    printf "%s\n" "${ips[@]}" >> "$BLOCK_FILE"
  done
}

# 🛡️ Apply block rules
block_ip() {
  local ip="$1" fw="$2"
  echo -e "${YELLOW}🔒 Blocking ${ip}...${NC}"
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

# 🔓 Remove block rules
unblock_ip() {
  local ip="$1" fw="$2"
  echo -e "${CYAN}🔓 Unblocking ${ip}...${NC}"
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

# 🚀 Main logic
main() {
  print_banner
  check_dependencies

  echo -e "${BOLD}${BLUE}🔍 Detecting firewall...${NC}"
  local fw=$(detect_firewall)
  if [[ "$fw" == "none" ]]; then
    echo -e "${RED}❌ No supported firewall detected. Exiting.${NC}"
    exit 1
  fi
  echo -e "${GREEN}✅ Detected firewall: ${fw}${NC}"

  if [[ "$1" == "--unblock" ]]; then
    if [[ -f "$BLOCK_FILE" ]]; then
      echo -e "${CYAN}🧼 Unblocking all IPs...${NC}"
      while read -r ip; do
        unblock_ip "$ip" "$fw"
      done < "$BLOCK_FILE"
      rm -f "$BLOCK_FILE"
      echo -e "${GREEN}✅ All IPs unblocked.${NC}"
    else
      echo -e "${YELLOW}⚠️ No blocked IPs found.${NC}"
    fi
    exit 0
  fi

  if [[ -f "$BLOCK_FILE" ]]; then
    echo -e "${CYAN}🧹 Cleaning up previous blocks...${NC}"
    while read -r ip; do
      unblock_ip "$ip" "$fw"
    done < "$BLOCK_FILE"
    rm -f "$BLOCK_FILE"
    echo -e "${GREEN}✅ Unblocked previous IPs.${NC}"
  fi

  echo -ne "${BLUE}🌐 Fetching server data...${NC}"
  fetch_data & spinner
  wait
  echo -e "\n${GREEN}✅ Data fetched.${NC}"

  echo -e "${YELLOW}📍 Available PoPs:${NC}"
  parse_countries

  if [[ $# -gt 0 ]]; then
    save_blocked_ips "$@"
  else
    echo -ne "${BOLD}Enter PoP codes to block (space-separated):${NC} "
    read -a countries
    save_blocked_ips "${countries[@]}"
  fi

  echo -e "${BLUE}🚫 Blocking IPs...${NC}"
  while read -r ip; do
    block_ip "$ip" "$fw"
  done < "$BLOCK_FILE"

  echo -e "${BOLD}${CYAN}📊 Blocked IPs Summary:${NC}"
  column -t "$BLOCK_FILE"

  echo -e "${GREEN}✅ Done.${NC}"
  rm -f "$TMP_DATA"
}

main "$@"
