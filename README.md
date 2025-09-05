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
   Replace `dxb bom2` with any PoP codes from the list that you want to block.  
   The script will fetch IPs and apply firewall rules automatically.

3. Unblock all IPs:  
   Re-run the script and press `Ctrl+C` when prompted for PoPs.  
   Previously blocked IPs will be removed.

## PoP Codes

- `hkg` → Hong Kong  
- `iad` → Washington, D.C.  
- `lax` → Los Angeles  
- `lhr` → London  

<details>
  <summary>View More</summary>

- `ams` → Amsterdam  
- `bom2` → Mumbai  
- `dxb` → Dubai  
- `fra` → Frankfurt  
- `mad` → Madrid  
- `man` → Manchester  
- `mrs` → Marseille  
- `osl` → Oslo  
- `par` → Paris  
- `scl` → Santiago  
- `sea` → Seattle  
- `sgp` → Singapore  
- `sto` → Stockholm  
- `syd` → Sydney  
- `tsn` → Tianjin  
- `vie` → Vienna  
- `waw` → Warsaw  

</details>

**Note:** Valve updates its server IPs from time to time. To keep your selected servers blocked, re-run the script occasionally to fetch the latest IPs and refresh the firewall rules.
