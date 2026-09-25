import json, urllib.request, time, base64
from pathlib import Path
import os, sys
url=os.environ['SOLANA_RPC_URL']
output=Path(sys.argv[1]) if len(sys.argv)>1 else Path(__file__).parent/'fixtures/mainnet-latest.json'
def rpc(method,params):
    for attempt in range(4):
        try:
            req=urllib.request.Request(url,json.dumps({'jsonrpc':'2.0','id':1,'method':method,'params':params}).encode(),{'Content-Type':'application/json'})
            data=json.load(urllib.request.urlopen(req,timeout=30))
            if 'error' in data: raise ValueError('RPC returned error')
            return data['result']
        except Exception:
            if attempt==3: raise RuntimeError('Read-only RPC failed for '+method) from None
            time.sleep(1)
programs={'pump':'6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P','pumpswap':'pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA'}
out={'captured_at':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'transactions':[],'accounts':[]}
for venue,program in programs.items():
    sigs=rpc('getSignaturesForAddress',[program,{'limit':100,'commitment':'confirmed'}])
    for sig in sigs:
        if sig['err'] is not None:continue
        tx=rpc('getTransaction',[sig['signature'],{'encoding':'json','commitment':'confirmed','maxSupportedTransactionVersion':1}])
        if tx and any(x.startswith('Program data: ') and (venue != 'pump' or base64.b64decode(x[14:])[:8] == bytes([189,219,127,211,78,230,97,238])) for x in tx['meta']['logMessages']):
            encoded=rpc('getTransaction',[sig['signature'],{'encoding':'base64','commitment':'confirmed','maxSupportedTransactionVersion':1}])
            out['transactions'].append({'venue':venue,'signature':sig['signature'],'json':tx,'encoded':encoded})
        if sum(t['venue']==venue for t in out['transactions'])>=5:break
for address,kind in [('Ef796zSZV6YHjCYSZJ4fzw5o1nb9Lquh1yWxAQsu9Kv9','curve'),('FphJwcwZsTKF4rUHe2Sy13Tmq4MYusNzYcsDsRjK1shC','pool')]:
    v=rpc('getAccountInfo',[address,{'encoding':'base64','commitment':'confirmed'}]);out['accounts'].append({'address':address,'kind':kind,**v})
output.write_text(json.dumps(out,indent=2)+'\n')
print(json.dumps({'captured_at':out['captured_at'],'transactions':len(out['transactions']),'slots':[t['encoded']['slot'] for t in out['transactions']],'accounts':[(a['kind'],a['context']['slot']) for a in out['accounts']]}))
