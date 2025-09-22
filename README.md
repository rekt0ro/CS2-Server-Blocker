## Usage

1. Make the script executable and run it:  
   ```bash
   chmod +x Block.sh
   ./Block.sh
   ```

2. Enter PoP codes shown under "Available PoPs".

   Script fetches IPs and applies UDP block rules automatically.

4. Unblock:

   ```bash
   ./Block.sh --unblock
   ```

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
