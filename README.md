## Requirements

- `curl` – fetches server data from Valve  
- `jq` – parses JSON

---

## Usage

1. Make the script executable and run it:  
   ```bash
   chmod +x Block.sh
   ./Block.sh
   ```

2. Block PoPs (datacenters):  
   ```bash
   ./Block.sh dxb bom2
   ```  
   Replace `dxb bom2` with any PoP codes you want to block.  
   The script will fetch IPs and apply firewall rules automatically.

3. View all available PoPs:  
   ```bash
   ./Block.sh
   ```

4. Unblock all IPs:  
   Re-run the script and press `Ctrl+C` when prompted for PoPs.  
   Previously blocked IPs will be removed.

---

## PoP Codes (Examples)

- `dxb` → Dubai  
- `bom2` → Mumbai  
- `fra` → Frankfurt  
- `ams` → Amsterdam  
- ...
