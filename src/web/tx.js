/* RadioMoon MPX Transmitter - web page */
const ABOUT={name:"RadioMoon MPX Transmitter",tagline:"FM multiplex generator with RDS, sent to sound cards and over IP",features:[
 "Stereo MPX with 19 kHz pilot and RDS: long PS, RadioText from the song, AF list, clock time with daylight saving, ECC",
 "Internet stream, sound input, AES67 / Dante or a ready MPX input (192 kHz) as the source",
 "Any number of IP receivers: UDP or TCP, L16 / L20 / L24, FLAC lossless or Compact MPX (FLAC / Opus)",
 "Protection: encryption with a password, FEC with burst protection; QoS (DSCP)",
 "SFN time stamps from GPS or PTP (IEEE 1588v2)",
 "Headless with this web interface (Linux, Raspberry Pi, Alpine), or with its window, as a service or a CLAP plug-in"]};
document.title="RadioMoon MPX Transmitter";$("#title").textContent="RadioMoon MPX Transmitter";
const TABS0=[["status","Status"],["source","Audio source"],["ip","IP output"],["card","Sound card output"],["signal","Signal"],["rds","RDS"],["sfn","SFN"],["account","Login"]];
function pills(){const p=$("#pills");p.innerHTML="";if(!ST){p.append(el("span",{class:"pill",style:"color:"+C.dim},"STOPPED"));return;}
 const ok=(ST.links||[]).some(([t,l])=>l.ok);p.append(el("span",{class:"pill",style:"color:"+(ok||ST.card?C.good:C.warn)},"ON AIR"));
 if(S&&S.mpx&&S.mpx.sfn_on&&ST.clock)p.append(" ",el("span",{class:"pill",style:"color:"+(ST.clock.synced?C.good:C.warn)},ST.clock.synced?"SFN CLOCK OK":"SFN CLOCK"));}
function clockFields(base){return [
 {p:base+".source",l:"Reference clock",t:"select",o:[["system","System clock (ptp4l / chrony)"],["ptp","PTP (IEEE 1588v2), built in"],["gps","GPS receiver (serial port)"]],re:1},
 {p:base+".ptp_domain",l:"PTP domain",t:"num",int:1,min:0,max:127,if:o=>get(o,base+".source")==="ptp"},
 {p:base+".ptp_iface",l:"Interface address",ph:"default",if:o=>get(o,base+".source")==="ptp"},
 {p:base+".gps_port",l:"Serial port",t:"select",o:()=>[["","Choose ..."],...DEV.serial.map(s=>[s,s])],if:o=>get(o,base+".source")==="gps"},
 {p:base+".gps_baud",l:"Baud rate",t:"select",o:[4800,9600,19200,38400,57600,115200].map(b=>[b,String(b)]),if:o=>get(o,base+".source")==="gps"},
 {p:base+".gps_pps",l:"PPS on the DCD line",t:"bool",re:1,if:o=>get(o,base+".source")==="gps"},
 {p:base+".gps_delay_ms",l:"NMEA delay",t:"num",suf:"ms",if:o=>get(o,base+".source")==="gps"&&!get(o,base+".gps_pps")}];}
const CODECS=[["flac","FLAC lossless (composite)"],["l24","PCM uncompressed (composite)"],["cflac","Compact MPX · FLAC (about 0.9 Mbit/s)"],["copus","Compact MPX · Opus (0.15 - 0.35 Mbit/s)"]];
function newTarget(){const last=S.mpx.targets[S.mpx.targets.length-1]||{};return {enabled:true,name:"Receiver "+(S.mpx.targets.length+1),host:"",port:last.port||5004,proto:last.proto||"udp",codec:last.codec||"flac",bits:24,dscp:46,opus_kbps:256,password:"",fec:0,fec_burst:1};}
function targetRow(t,i,re){
 const live=((ST&&ST.links)||[]).find(([lt])=>lt.host===t.host&&lt.port===t.port&&lt.proto===t.proto&&lt.codec===t.codec);
 const f=[{p:"enabled",l:"Send",t:"bool"},{p:"name",l:"Name",ph:"site"},{p:"host",l:"Address",ph:"IP or host name"},{p:"port",l:"Port",t:"num",int:1,min:1,max:65535},
  {p:"proto",l:"Protocol",t:"select",o:[["udp","UDP (RTP)"],["tcp","TCP"]],re:1},{p:"codec",l:"Format",t:"select",o:CODECS,re:1},
  {p:"bits",l:"Bits",t:"select",o:[[24,"24-bit"],[20,"20-bit"],[16,"16-bit"]],if:o=>o.codec==="flac"||o.codec==="l24"},
  {p:"opus_kbps",l:"Opus bit rate",t:"select",o:[[320,"320 kbit/s"],[256,"256 kbit/s (recommended)"],[192,"192 kbit/s"],[128,"128 kbit/s"]],if:o=>o.codec==="copus"},
  {p:"dscp",l:"QoS (DSCP)",t:"select",o:[[46,"EF (46), recommended"],[34,"AF41 (34)"],[40,"CS5 (40)"],[48,"CS6 (48)"],[0,"Off"]]},
  {p:"password",l:"Password",t:"pw",h:"Encrypts the stream; the receiver needs the same password. Empty: not encrypted."},
  {p:"fec",l:"FEC",t:"select",o:[[0,"Off"],[20,"1 per 20 packets (+5%)"],[10,"1 per 10 packets (+10%)"],[5,"1 per 5 packets (+20%)"]],re:1,if:o=>o.proto!=="tcp"},
  {p:"fec_burst",l:"Burst protection",t:"select",o:[[1,"Single losses"],[2,"Bursts of 2"],[4,"Bursts of 4"],[8,"Bursts of 8"]],if:o=>o.proto!=="tcp"&&o.fec>0}];
 const rm=el("button",{class:"s",style:"float:right",onclick:()=>{S.mpx.targets.splice(i,1);dirty=true;re();}},"Remove");
 const head=el("div",{style:"margin-bottom:6px"},dot(live?(live[1].ok?C.good:C.bad):C.dim),el("b",{},t.name||t.host||"Receiver"),live?el("span",{class:"hint"}," · "+mbps(live[1].mbps)+" Mbit/s · "+live[1].text):null,rm);
 return el("div",{class:"row"},head,form(f,t,re));}
function render(){const m=$("#main");m.innerHTML="";const re=()=>render();
 if(TAB==="status"){
  if(!ST){m.append(card("Status",el("p",{},"The Transmitter is not running.")));return;}
  m.append(card("Transmitter",el("div",{class:"tiles"},tile("Peak",(ST.peak*100).toFixed(0),"%",ST.peak>0.98?C.bad:C.accent),tile("IP outputs",(ST.links||[]).filter(([t,l])=>l.ok).length+" / "+(ST.links||[]).length,"sending"),tile("Total",mbps((ST.links||[]).reduce((a,[t,l])=>a+l.mbps,0)),"Mbit/s",C.accent)),
   el("p",{},dot(C.good),ST.status),el("div",{class:"hint"},"Source: "+ST.source),ST.now?el("div",{},"Now: ",el("b",{},ST.now)):null,ST.card?el("div",{class:"hint"},"Sound card: "+ST.card):null,
   ST.clock?el("div",{class:"hint"},dot(ST.clock.synced?C.good:C.warn),"Clock: "+ST.clock.text):null));
  const tb=el("tbody");for(const [t,l] of ST.links||[])tb.append(el("tr",{},el("td",{},dot(l.ok?C.good:C.bad),t.name||t.host),el("td",{class:"hint"},t.host+":"+t.port),el("td",{style:"white-space:nowrap"},mbps(l.mbps)+" Mbit/s"),el("td",{class:"hint"},l.text)));
  if((ST.links||[]).length)m.append(card("IP outputs",el("table",{},el("thead",{},el("tr",{},el("th",{},"Receiver"),el("th",{},"Address"),el("th",{},"Bandwidth"),el("th",{},"State"))),tb)));
  const cv=el("canvas");m.append(card("Signal",cv,levels(ST.scope)));requestAnimationFrame(()=>spectrum(cv,ST.scope));
 }
 else if(TAB==="source"){
  m.append(card("Audio source",form([{p:"kind",l:"Source",t:"select",o:[["stream","Internet radio stream"],["input","Sound input (audio)"],["aes67","AES67 / Dante"],["mpx","MPX input (192 kHz, ready composite)"]],re:1},
   {p:"url",l:"Stream address",ph:"http://server:8000/stream",if:o=>o.kind==="stream",h:"The song titles of the stream become the RadioText."},
   {p:"device",l:"Sound input",t:"select",o:devs(()=>DEV.inputs),if:o=>o.kind==="input"},
   {p:"aes67.group",l:"Multicast address",ph:"239.69.x.x (empty = unicast)",if:o=>o.kind==="aes67"},{p:"aes67.port",l:"Port",t:"num",int:1,if:o=>o.kind==="aes67"},
   {p:"aes67.iface",l:"Interface address",ph:"default",if:o=>o.kind==="aes67"},{p:"aes67.encoding",l:"Format",t:"select",o:[["L24","L24"],["L16","L16"]],if:o=>o.kind==="aes67"},
   {p:"aes67.rate",l:"Sample rate",t:"select",o:[[44100,"44.1 kHz"],[48000,"48 kHz"],[96000,"96 kHz"]],if:o=>o.kind==="aes67"},{p:"aes67.channels",l:"Channels in the stream",t:"num",int:1,min:1,max:64,if:o=>o.kind==="aes67"},
   {p:"aes67.ch_l",l:"Left = channel",t:"num",int:1,min:0,h:"0 = the first",if:o=>o.kind==="aes67"},{p:"aes67.ch_r",l:"Right = channel",t:"num",int:1,min:0,if:o=>o.kind==="aes67"},
   {p:"mpx_device",l:"MPX sound input",t:"select",o:devs(()=>DEV.inputs),if:o=>o.kind==="mpx"},{p:"mpx_channel",l:"Channel",t:"select",o:[[0,"Left"],[1,"Right"]],if:o=>o.kind==="mpx"},
   {p:"gain_db",l:"Level",t:"num",step:0.1,suf:"dB"}],S,re)));
  m.append(saveBar());
 }
 else if(TAB==="ip"){
  const box=el("div");S.mpx.targets.forEach((t,i)=>box.append(targetRow(t,i,re)));
  m.append(card("IP output",form([{p:"mpx.ip_on",l:"Send over IP",t:"bool"}],S,re),el("div",{style:"margin-top:8px"},box),
   el("button",{class:"s",onclick:()=>{S.mpx.targets.push(newTarget());dirty=true;re();}},"+ Add receiver")));
  m.append(saveBar());
 }
 else if(TAB==="card"){
  m.append(card("Sound card output (192 kHz)",form([{p:"mpx.card_on",l:"Play the MPX on a sound card",t:"bool",re:1},{p:"mpx.card_device",l:"Sound device",t:"select",o:devs(()=>DEV.outputs),if:o=>o.mpx.card_on}],S,re)));
  m.append(saveBar());
 }
 else if(TAB==="signal"){
  m.append(card("Signal",form([{p:"mpx.stereo",l:"Stereo",t:"bool"},{p:"mpx.audio_level",l:"Audio level",t:"num",step:0.01,min:0,max:1.2,suf:"(1.0 = 75 kHz)"},{p:"mpx.pilot_level",l:"Pilot level",t:"num",step:0.005,min:0,max:0.2,suf:"(0.09 = 6.75 kHz)"},
   {p:"mpx.rds_level",l:"RDS level",t:"num",step:0.005,min:0,max:0.15,suf:"(0.04 = 3 kHz)"},{p:"mpx.preemph_us",l:"Pre-emphasis",t:"select",o:[[50,"50 µs (Europe)"],[75,"75 µs (Americas)"],[0,"Off"]]},
   {p:"mpx.limiter",l:"Limiter",t:"bool"},{p:"mpx.output",l:"Output level",t:"num",step:0.01,min:0,max:2}],S,re),el("p",{class:"hint"},"Not used with an MPX input (the composite is sent as it is).")));
  m.append(saveBar());
 }
 else if(TAB==="rds"){
  m.append(card("RDS",form([{p:"mpx.rds_on",l:"Send RDS",t:"bool"},{p:"mpx.pi",l:"PI code",ph:"1234"},{p:"mpx.ecc",l:"ECC (country)",ph:"E1",h:"Extended Country Code, hex: Greece E1, Cyprus E2. Empty: not sent. Needed by RadioDNS hybrid radio in cars."},{p:"mpx.ps",l:"Station name (PS)",ph:"RADIOMON",h:"8 characters, or a longer text shown in pages"},
   {p:"mpx.ps_mode",l:"Long name",t:"select",o:[["static","First 8 characters"],["pages","In pages (whole words)"],["scroll","Scrolling"]]},{p:"mpx.ps_interval",l:"Seconds per page",t:"num",step:0.5,min:1},
   {p:"mpx.pty",l:"Programme type (PTY)",t:"num",int:1,min:0,max:31},{p:"mpx.tp",l:"TP",t:"bool"},{p:"mpx.ta",l:"TA",t:"bool"},{p:"mpx.music",l:"Music",t:"bool"},
   {p:"mpx.rt",l:"RadioText",ph:"RadioMoon"},{p:"mpx.rt_from_playlist",l:"RadioText from the stream's titles",t:"bool"},
   {p:"mpx.af",l:"Alternative frequencies",ph:"94.3, 101.5 or 943 1015",h:"Up to 25 (87.6 - 107.9 MHz)."},
   {p:"mpx.ct_on",l:"Clock time (CT)",t:"bool",re:1},{p:"mpx.ct_offset_h",l:"Local time offset",t:"num",step:0.5,min:-12,max:14,suf:"h (standard / winter time)",if:o=>o.mpx.ct_on},
   {p:"mpx.ct_dst",l:"Daylight saving",t:"select",o:[["eu","Automatic (Europe)"],["us","Automatic (USA / Canada)"],["off","Off (fixed offset)"]],if:o=>o.mpx.ct_on}],S,re)));
  m.append(saveBar());
 }
 else if(TAB==="sfn"){
  m.append(card("SFN (single frequency network)",form([{p:"mpx.sfn_on",l:"Send SFN time stamps",t:"bool",re:1,h:"In every UDP packet: the UTC time of its first sample, from the reference clock."},...clockFields("mpx.clock").map(f=>Object.assign({},f,{if:o=>o.mpx.sfn_on&&(!f.if||f.if(o))}))],S,re)));
  m.append(saveBar());
 }
 else if(TAB==="account")m.append(accountCard());
}
