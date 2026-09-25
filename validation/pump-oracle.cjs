/** Offline independent oracle. Uses only public captured chain data; no RPC or keys. */
const fs = require('node:fs');
const path = require('node:path');
const modules = process.env.ORACLE_NODE_MODULES || path.join(__dirname, 'node_modules');
const load = name => require(path.join(modules, name));
const pump = load('@pump-fun/pump-sdk');
const amm = load('@pump-fun/pump-swap-sdk');
const { BorshEventCoder } = load('@coral-xyz/anchor');
const { PublicKey } = load('@solana/web3.js');
const BN = load('bn.js');
const input = process.argv[2] || path.join(__dirname, 'fixtures/mainnet-2026-09-25.json');
const corpus = JSON.parse(fs.readFileSync(input));
function normalize(value) {
  if (BN.isBN(value)) return value.toString(10);
  if (value instanceof PublicKey) return [...value.toBytes()];
  if (Array.isArray(value)) return value.map(normalize);
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k,v]) => [k.replace(/[A-Z]/g, m=>'_'+m.toLowerCase()), normalize(v)]));
  return value;
}
const coders = {
  pump: new BorshEventCoder(pump.pumpIdl),
  pumpswap: amm.OFFLINE_PUMP_AMM_PROGRAM.coder.events,
};
const expected = {versions: Object.fromEntries(['pump-sdk','pump-swap-sdk'].map(n=>[n,load('@pump-fun/'+n+'/package.json').version])),transactions:[],accounts:[]};
for (const tx of corpus.transactions) {
  const events = [];
  for (const log of tx.json.meta.logMessages) {
    if (!log.startsWith('Program data: ')) continue;
    let event;
    try { event = coders[tx.venue].decode(log.slice(14)); } catch { continue; }
    if (!event || !['TradeEvent','tradeEvent','BuyEvent','buyEvent','SellEvent','sellEvent'].includes(event.name)) continue;
    const data = normalize(event.data);
    const kind = tx.venue === 'pump' ? (data.is_buy ? 'PumpFunBuy' : 'PumpFunSell') : (/buy/i.test(event.name) ? 'PumpSwapBuy' : 'PumpSwapSell');
    const fields = tx.venue === 'pump' ? ['mint','user','sol_amount','token_amount','is_buy','virtual_sol_reserves','virtual_token_reserves','real_sol_reserves','real_token_reserves','fee_basis_points','fee','creator_fee_basis_points','creator_fee','ix_name'] : ['pool','user','base_amount_out','quote_amount_in','base_amount_in','quote_amount_out','pool_base_token_reserves','pool_quote_token_reserves','lp_fee_basis_points','lp_fee','protocol_fee_basis_points','protocol_fee','coin_creator_fee_basis_points','coin_creator_fee','virtual_quote_reserves'];
    events.push({kind,fields:Object.fromEntries(fields.filter(k=>k in data).map(k=>[k,data[k]]))});
  }
  if (!events.length) throw new Error('No official trade events for '+tx.signature);
  expected.transactions.push({signature:tx.signature,slot:tx.encoded.slot,events});
}
for(const account of corpus.accounts) {
  const original = Buffer.from(account.value.data[0],'base64');
  const lengths = account.kind === 'curve' ? [49,81,82,83,115,123,124,125,151,256] : [211,243,244,245,252,261,269,270,271,300,301];
  for(const length of [...new Set([...lengths,original.length])]) {
    const data = Buffer.alloc(length);original.copy(data,0,0,Math.min(length,original.length));
    if(account.kind==='pool' && length===252) data.fill(0,245);
    const decoded = account.kind==='curve' ? pump.PUMP_SDK.decodeBondingCurve({data}) : amm.PUMP_AMM_SDK.decodePool({data});
    expected.accounts.push({address:account.address,kind:account.kind,length,data:data.toString('base64'),fields:normalize(decoded)});
  }
}
const output=input.replace(/\.json$/,'.expected.json');fs.writeFileSync(output,JSON.stringify(expected,null,2)+'\n');
console.log(JSON.stringify({output,transactions:expected.transactions.length,events:expected.transactions.reduce((n,t)=>n+t.events.length,0),accountCases:expected.accounts.length,versions:expected.versions}));
