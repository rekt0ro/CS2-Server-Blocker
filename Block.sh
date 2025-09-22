#!/usr/bin/env bash
set -euo pipefail

API_ENDPOINT="https://api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730"
BLOCK_FILE="blocked-ips.txt"
TMP_DATA="/tmp/cs2_data.json"

BOLD='\033[1m'; DIM='\033[2m'; NC='\033[0m'
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; MAGENTA='\033[0;35m'

cleanup(){ rm -f "$TMP_DATA" >/dev/null 2>&1 || true; }
trap cleanup EXIT

spinner(){
  local pid=$1 delay=0.08 i=0 spin='|/-\' ch
  tput civis 2>/dev/null || true
  while kill -0 "$pid" >/dev/null 2>&1; do
    ch="${spin:i%4:1}"
    printf " %b[%c]%b" "$CYAN" "$ch" "$NC"
    sleep "$delay"
    printf "\b\b\b"
    i=$((i+1))
  done
  printf "    \b\b\b\b"
  tput cnorm 2>/dev/null || true
}

print_header(){
  local title="CS2 Server Blocker"
  if command -v figlet >/dev/null 2>&1; then
    figlet -f slant "$title" | sed "s/^/ /"
  elif command -v toilet >/dev/null 2>&1; then
    toilet -f mono12 -F metal "$title" | sed "s/^/ /"
  else
    printf "\n"
    printf "%b%s%b\n" "${CYAN}${BOLD}" "$title" "${NC}"
    local ul
    ul=$(printf '%*s' $(( ${#title} + 6 )) '' | tr ' ' '=')
    printf "%b%s%b\n\n" "${DIM}" "$ul" "${NC}"
  fi
}

check_deps(){
  for d in curl jq column awk tput; do
    if ! command -v "$d" >/dev/null 2>&1; then
      echo -e "${RED}Missing dependency: ${d}${NC}. Install it and re-run." >&2
      exit 1
    fi
  done
}

detect_firewall(){
  if command -v ufw >/dev/null 2>&1 && sudo ufw status >/dev/null 2>&1; then
    echo "ufw"
  elif command -v firewall-cmd >/dev/null 2>&1 && sudo firewall-cmd --state >/dev/null 2>&1; then
    echo "firewalld"
  elif command -v nft >/dev/null 2>&1 && sudo nft list ruleset >/dev/null 2>&1; then
    echo "nftables"
  elif command -v iptables >/dev/null 2>&1 && sudo iptables -L >/dev/null 2>&1; then
    echo "iptables"
  else
    echo "none"
  fi
}

fetch_data(){
  if ! curl -s --fail "$API_ENDPOINT" -o "$TMP_DATA"; then
    echo -e "${RED}Failed to fetch Valve data${NC}" >&2
    return 1
  fi
  if ! jq empty < "$TMP_DATA" 2>/dev/null; then
    echo -e "${RED}Invalid JSON from Valve${NC}" >&2
    return 1
  fi
  jq 'del(.success, .certs, .p2p_share_ip, .relay_public_key, .revoked_keys, .typical_pings) // .' "$TMP_DATA" > "${TMP_DATA}.cleaned"
  mv "${TMP_DATA}.cleaned" "$TMP_DATA"
}

get_pop_list(){ jq -r '.pops | keys[]' < "$TMP_DATA" 2>/dev/null | sort; }

display_pops_horizontal(){
  echo
  echo -e "${MAGENTA}${BOLD}Available PoPs${NC}"
  echo
  mapfile -t pops < <(get_pop_list)
  (( ${#pops[@]} == 0 )) && { echo -e "${YELLOW}No PoPs found.${NC}"; return; }
  local pad=10 cols=6 tw
  tw=$(tput cols 2>/dev/null || echo 80)
  while (( cols>1 && pad*cols > tw )); do cols=$((cols-1)); done
  local i=0
  for p in "${pops[@]}"; do
    printf " %b%-${pad}s%b" "$CYAN" "$p" "$NC"
    i=$((i+1))
    if (( i % cols == 0 )); then printf "\n"; fi
  done
  (( i % cols != 0 )) && printf "\n"
  echo
}

get_ips_by_pop(){ local pop="$1"; jq -r --arg p "$pop" '.pops[$p].relays[]?.ipv4' < "$TMP_DATA" 2>/dev/null || true; }

save_blocked_ips(){
  : > "$BLOCK_FILE"
  local added=0
  for pop in "$@"; do
    mapfile -t ips < <(get_ips_by_pop "$pop")
    if (( ${#ips[@]} > 0 )); then
      printf '%s\n' "${ips[@]}" >> "$BLOCK_FILE"
      added=$((added + ${#ips[@]}))
      echo -e "  ${GREEN}•${NC} ${BOLD}${pop}${NC} -> ${DIM}${#ips[@]} IPs${NC}"
    else
      echo -e "  ${YELLOW}•${NC} ${BOLD}${pop}${NC} -> ${RED}no IPs${NC}"
    fi
  done
  echo
  printf "  %bTotal IPs queued:%b %s\n\n" "$BLUE" "$NC" "$added"
}

block_ip(){
  local ip="$1" fw="$2"
  case "$fw" in
    ufw)
      sudo ufw deny out to "$ip" proto udp >/dev/null 2>&1 || true
      sudo ufw deny from "$ip" proto udp >/dev/null 2>&1 || true
      ;;
    firewalld)
      sudo firewall-cmd --permanent --add-rich-rule="rule family='ipv4' source address='$ip' protocol='udp' drop" >/dev/null 2>&1 || true
      sudo firewall-cmd --permanent --add-rich-rule="rule family='ipv4' destination address='$ip' protocol='udp' drop" >/dev/null 2>&1 || true
      ;;
    nftables)
      sudo nft add rule inet filter input ip saddr "$ip" udp drop >/dev/null 2>&1 || true
      sudo nft add rule inet filter output ip daddr "$ip" udp drop >/dev/null 2>&1 || true
      ;;
    iptables)
      sudo iptables -I INPUT -s "$ip" -p udp -j DROP >/dev/null 2>&1 || true
      sudo iptables -I OUTPUT -d "$ip" -p udp -j DROP >/dev/null 2>&1 || true
      ;;
    *)
      ;;
  esac
}

unblock_ip(){
  local ip="$1" fw="$2"
  case "$fw" in
    ufw)
      sudo ufw delete deny out to "$ip" proto udp >/dev/null 2>&1 || true
      sudo ufw delete deny from "$ip" proto udp >/dev/null 2>&1 || true
      ;;
    firewalld)
      sudo firewall-cmd --permanent --remove-rich-rule="rule family='ipv4' source address='$ip' protocol='udp' drop" >/dev/null 2>&1 || true
      sudo firewall-cmd --permanent --remove-rich-rule="rule family='ipv4' destination address='$ip' protocol='udp' drop" >/dev/null 2>&1 || true
      ;;
    nftables)
      sudo nft delete rule inet filter input ip saddr "$ip" udp drop >/dev/null 2>&1 || true
      sudo nft delete rule inet filter output ip daddr "$ip" udp drop >/dev/null 2>&1 || true
      ;;
    iptables)
      sudo iptables -D INPUT -s "$ip" -p udp -j DROP >/dev/null 2>&1 || true
      sudo iptables -D OUTPUT -d "$ip" -p udp -j DROP >/dev/null 2>&1 || true
      ;;
  esac
}

progress_bar(){
  local total=$1 done=0 width=40 pct bar
  while read -r line; do
    done=$((done+1))
    pct=$((done*100/total))
    local filled=$((width*pct/100))
    bar="$(printf '%0.s#' $(seq 1 $filled) 2>/dev/null)$(printf '%0.s-' $(seq 1 $((width-filled))) 2>/dev/null)"
    printf "\r %bBlocking%b [%s] %3d%%" "$MAGENTA" "$NC" "$bar" "$pct"
  done
  printf "\n"
}

unblock_all_and_exit(){
  local fw="$1"
  if [[ ! -f "$BLOCK_FILE" || ! -s "$BLOCK_FILE" ]]; then
    echo -e "${YELLOW}No blocked IPs file found (${BLOCK_FILE}). Nothing to do.${NC}"
    exit 0
  fi
  echo -e "${BOLD}Unblocking all IPs from ${BLOCK_FILE}...${NC}"
  while IFS= read -r ip; do
    [[ -z "$ip" ]] && continue
    unblock_ip "$ip" "$fw"
    echo -e "  ${GREEN}✓${NC} Unblocked ${ip}"
  done < "$BLOCK_FILE"
  rm -f "$BLOCK_FILE"
  echo -e "${GREEN}All entries removed and firewall rules cleaned.${NC}"
  exit 0
}

main(){
  print_header
  check_deps

  echo -e "${BLUE}Detecting firewall...${NC}"
  local fw
  fw=$(detect_firewall)
  if [[ "$fw" == "none" ]]; then
    echo -e "${RED}No supported firewall detected. Exiting.${NC}" >&2
    exit 1
  fi
  echo -e "${GREEN}Detected firewall:${NC} ${BOLD}$fw${NC}"
  echo

  if [[ "${1:-}" == "--unblock" ]]; then
    unblock_all_and_exit "$fw"
  fi

  echo -ne "${CYAN}Fetching server data...${NC}"
  fetch_data & fetch_pid=$!
  spinner "$fetch_pid"
  wait "$fetch_pid"
  echo -e " ${GREEN}done${NC}\n"

  display_pops_horizontal

  echo -e "${BOLD}Enter PoP codes to block (space-separated).${NC}"
  echo -e "Examples: ${CYAN}ams atl fra${NC}    |    Press ${DIM}Enter${NC} to cancel"
  printf "%b→ %b" "$MAGENTA" "$NC"
  read -r -a picks
  if [[ ${#picks[@]} -eq 0 ]]; then
    echo -e "${YELLOW}No PoP codes entered. Exiting.${NC}"
    exit 0
  fi
  echo

  save_blocked_ips "${picks[@]}"
  if [[ ! -s "$BLOCK_FILE" ]]; then
    echo -e "${YELLOW}No IPs found for selected PoPs. Exiting.${NC}"
    exit 0
  fi

  local total_ips
  total_ips=$(wc -l < "$BLOCK_FILE" | tr -d ' ')
  echo -e "${BOLD}Ready to block${NC}: ${CYAN}$total_ips${NC} IPs from ${CYAN}${#picks[@]}${NC} PoP(s)."
  printf "%bProceed? [y/N] %b" "$YELLOW" "$NC"
  read -r confirm
  if [[ "${confirm,,}" != "y" ]]; then
    echo -e "${YELLOW}Aborted.${NC}"
    exit 0
  fi

  echo
  awk '{ print }' "$BLOCK_FILE" | ( progress_bar "$total_ips" ) &
  pb_pid=$!

  while IFS= read -r ip; do
    [[ -z "$ip" ]] && printf "\n" || {
      block_ip "$ip" "$fw"
      printf "%s\n" "$ip"
    }
  done < "$BLOCK_FILE" > /proc/$$/fd/1 || {
    while IFS= read -r ip; do
      block_ip "$ip" "$fw"
      printf "%s\n" "$ip"
    done < "$BLOCK_FILE"
  }

  wait "$pb_pid" 2>/dev/null || true

  echo -e "\n${GREEN}Done.${NC} Blocked ${CYAN}$total_ips${NC} IPs from ${CYAN}${#picks[@]}${NC} PoP(s)."
  echo
}

main "$@"
