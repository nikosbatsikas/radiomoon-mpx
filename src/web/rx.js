/* RadioMoon MPX Receiver - web page */
const ABOUT={name:"RadioMoon MPX Receiver",tagline:"Receives the MPX over IP and plays it into the exciter",features:[
 "UDP (RTP) or TCP, format detected automatically: L16 / L20 / L24, FLAC and Compact MPX (FLAC / Opus)",
 "Lost packets hidden without a pilot or RDS phase jump; FEC restores lost packets",
 "Encrypted streams (password) and a list of allowed senders",
 "Backup MPX from a sound card input when the stream stops",
 "RDS display: station name, RadioText, PI, ECC, PTY, AF, clock time",
 "SFN playout with GPS or PTP (IEEE 1588v2) time; event log",
 "Headless with this web interface (Linux, Raspberry Pi, Alpine), or with its window, as a service"]};
document.title="RadioMoon MPX Receiver";$("#title").textContent="RadioMoon MPX Receiver";
const TABS0=[["status","Status"],["log","Log"],["input","Input & protection"],["output","Output"],["sfn","SFN"],["backup","Backup"],["account","Login"]];
function fmtName(st){const b=st.bits||24;if(st.compact===1)return "Compact MPX (FLAC)";if(st.compact===2)return "Compact MPX (Opus)";return st.flac?"FLAC "+b+"-bit":"L"+b;}
function pills(){const p=$("#pills");p.innerHTML="";if(!ST||!ST.view){p.append(el("span",{class:"pill",style:"color:"+C.dim},"STOPPED"));return;}
 const v=ST.view;const add=(t,c)=>p.append(el("span",{class:"pill",style:"color:"+c+";margin-right:6px"},t));
 add(v.receiving?"RECEIVING":"NO SIGNAL",v.receiving?C.good:C.warn);
 if(v.backup&&v.backup.active)add("ON BACKUP",C.warn);
 if(v.receiving&&v.protect&&v.protect.encrypted)add("ENCRYPTED",C.good);
 if(v.sfn)add(v.sfn.locked?"SFN LOCKED":"SFN",v.sfn.locked?C.good:C.warn);}
function clockFields(base){return [
 {p:base+".source",l:"Reference clock",t:"select",o:[["system","System clock (ptp4l / chrony)"],["ptp","PTP (IEEE 1588v2), built in"],["gps","GPS receiver (serial port)"]],re:1},
 {p:base+".ptp_domain",l:"PTP domain",t:"num",int:1,min:0,max:127,if:o=>get(o,base+".source")==="ptp"},
 {p:base+".ptp_iface",l:"Interface address",ph:"default",if:o=>get(o,base+".source")==="ptp"},
 {p:base+".gps_port",l:"Serial port",t:"select",o:()=>[["","Choose ..."],...DEV.serial.map(s=>[s,s])],if:o=>get(o,base+".source")==="gps"},
 {p:base+".gps_baud",l:"Baud rate",t:"select",o:[4800,9600,19200,38400,57600,115200].map(b=>[b,String(b)]),if:o=>get(o,base+".source")==="gps"},
 {p:base+".gps_pps",l:"PPS on the DCD line",t:"bool",re:1,if:o=>get(o,base+".source")==="gps"},
 {p:base+".gps_delay_ms",l:"NMEA delay",t:"num",suf:"ms",if:o=>get(o,base+".source")==="gps"&&!get(o,base+".gps_pps")}];}
function render(){const m=$("#main");m.innerHTML="";const re=()=>render();
 if(TAB==="status"){
  if(!ST||!ST.view){m.append(card("Status",el("p",{},"The Receiver is not running.")));return;}
  const v=ST.view,st=v.stats,lost=st.lost+st.bad;
  m.append(card("Link",el("div",{class:"tiles"},tile("Bandwidth",mbps(v.mbps),"Mbit/s",C.accent),tile("Format",v.receiving?fmtName(st):"-",""),tile("Buffer",v.buffer_ms,"ms"),tile("Lost packets",lost,(st.concealed/192).toFixed(0)+" ms hidden",lost?C.warn:C.good),tile("Received",v.total_mb.toFixed(1),"MB")),
   el("p",{},dot(v.receiving?C.good:C.warn),ST.status),v.card?el("div",{class:"hint"},v.card):null));
  const r=v.rds;m.append(card("RDS",r&&r.synced?el("div",{},el("span",{class:"ps"},r.ps||"        ")," ",el("span",{class:"hint"},"PI "+(r.pi!=null?r.pi.toString(16).toUpperCase().padStart(4,"0"):"-")+(r.ecc!=null?" · ECC "+r.ecc.toString(16).toUpperCase():"")+" · PTY "+(r.pty!=null?r.pty:"-")+" · "+Math.round(r.quality*100)+"% blocks ok"),
   el("p",{},r.rt||"-"),(r.af&&r.af.length)?el("div",{class:"hint"},"AF "+r.af.map(f=>f.toFixed(1)).join("  ")+" MHz"):null,r.ct?el("div",{class:"hint"},"CT "+r.ct):null):el("p",{class:"hint"},v.receiving?"No RDS in the signal":"No signal")));
  const p=v.protect||{};const pl=[];if(v.receiving)pl.push([p.encrypted?C.good:C.dim,p.encrypted?"Encrypted, password correct":"Not encrypted"]);
  if(p.fec_n)pl.push([p.lost?C.warn:C.good,"FEC 1 per "+p.fec_n+(p.fec_depth>1?", bursts of "+p.fec_depth:"")+": restored "+p.restored+", could not restore "+p.lost]);
  if(p.need_password)pl.push([C.warn,"An encrypted stream arrives: enter its password"]);if(p.rejected)pl.push([C.bad,p.rejected+" packets rejected (wrong password)"]);
  if(p.plain_rejected)pl.push([C.warn,p.plain_rejected+" unencrypted packets ignored"]);if(p.blocked)pl.push([C.warn,p.blocked+" blocked, last from "+p.blocked_from]);
  const b=v.backup||{};if(b.enabled)pl.push([b.active?C.warn:C.good,b.active?"BACKUP ON AIR"+(v.receiving?" - stream back on air in "+b.return_in_s.toFixed(0)+" s":""):"Stream on air, backup ready ("+(b.level*100).toFixed(0)+"% input level, switched "+b.switches+" times)"]);
  if(v.sfn){const s=v.sfn;pl.push([s.locked?C.good:C.warn,"SFN: "+s.state+" · error "+s.error_us.toFixed(1)+" µs · "+s.ppm.toFixed(2)+" ppm · margin "+s.margin_ms.toFixed(0)+" ms"]);}
  if(v.clock)pl.push([v.clock.synced?C.good:C.warn,"Clock: "+v.clock.text]);
  if(pl.length)m.append(card("Protection, backup, SFN",...pl.map(([c,t])=>el("div",{},dot(c),t))));
  const cv=el("canvas");m.append(card("Signal",cv,levels(ST.scope)));requestAnimationFrame(()=>spectrum(cv,ST.scope));
 }
 else if(TAB==="log"){
  const ev=(ST&&ST.view&&ST.view.events)||[];const tb=el("tbody");
  for(const e of ev.slice().reverse()){tb.append(el("tr",{},el("td",{},dot(e.level===2?C.bad:e.level===1?C.warn:C.good)),el("td",{class:"mono",style:"white-space:nowrap;color:"+C.dim},new Date(e.t).toLocaleString()),el("td",{style:e.level===2?"color:"+C.bad:""},e.text)));}
  m.append(card("Log (newest first, local time)",el("p",{class:"hint"},ev.length+" events since the start. Lost packets are summed up every 10 seconds."),el("table",{},el("thead",{},el("tr",{},el("th",{},""),el("th",{},"Time"),el("th",{},"Event"))),tb)));
 }
 else if(TAB==="input"){
  m.append(card("Input (from the transmitter)",form([{p:"proto",l:"Protocol",t:"select",o:[["udp","UDP (RTP)"],["tcp","TCP"]]},{p:"port",l:"Port",t:"num",int:1,min:1,max:65535}],S,re),el("p",{class:"hint"},"The format is detected automatically: L16 / L20 / L24, FLAC or Compact MPX.")));
  m.append(card("Protection",form([{p:"password",l:"Password",t:"pw",h:"Empty: only unencrypted streams play. Set: only the stream encrypted with this password plays."},{p:"allow",l:"Allowed senders",ph:"everyone, or 192.168.1.10, 10.0.0.0/8",h:"Addresses or networks; packets and connections from others are refused."}],S,re)));
  m.append(saveBar());
 }
 else if(TAB==="output"){
  m.append(card("Output (exciter MPX input)",form([{p:"device",l:"Sound device (192 kHz)",t:"select",o:devs(()=>DEV.outputs)},{p:"buffer_ms",l:"Buffer",t:"num",int:1,min:20,max:2000,suf:"ms",if:o=>!o.sfn_on}],S,re)));
  m.append(saveBar());
 }
 else if(TAB==="sfn"){
  m.append(card("SFN (single frequency network)",form([{p:"sfn_on",l:"Play at time stamp + delay",t:"bool",re:1},{p:"sfn_delay_ms",l:"Delay",t:"num",suf:"ms (the same at every site)",if:o=>o.sfn_on},{p:"sfn_correction_us",l:"Output correction",t:"num",suf:"µs (this site's sound card, exciter, cables)",if:o=>o.sfn_on},...clockFields("clock").map(f=>Object.assign({},f,{if:o=>o.sfn_on&&(!f.if||f.if(o))}))],S,re),
   el("p",{class:"hint"},"UDP only. The Transmitter must send SFN time stamps.")));
  m.append(saveBar());
 }
 else if(TAB==="backup"){
  m.append(card("Backup source (MPX from a sound card input)",form([{p:"backup.on",l:"Use a backup",t:"bool",re:1,h:"When the stream stops, the MPX of a sound input goes on air (analog STL, off-air receiver, local processor)."},
   {p:"backup.device",l:"Sound input (192 kHz)",t:"select",o:devs(()=>DEV.inputs),if:o=>o.backup.on},{p:"backup.channel",l:"Channel",t:"select",o:[[0,"Left"],[1,"Right"]],if:o=>o.backup.on},
   {p:"backup.gain_db",l:"Level",t:"num",step:0.1,suf:"dB",if:o=>o.backup.on},{p:"backup.timeout_ms",l:"Switch to backup after",t:"num",int:1,min:100,suf:"ms without stream",if:o=>o.backup.on},
   {p:"backup.return_s",l:"Back to the stream after",t:"num",min:0,suf:"s of steady stream",if:o=>o.backup.on},{p:"backup.on_silence",l:"Also on silence",t:"bool",h:"Switch also when the stream carries silence (MPX below -40 dB).",if:o=>o.backup.on}],S,re)));
  m.append(saveBar());
 }
 else if(TAB==="account")m.append(accountCard());
}
