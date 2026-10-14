// Dependency-free browser smoke tests using Node's WebSocket and Chromium CDP.
import http from 'node:http';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {spawn} from 'node:child_process';

const args=JSON.parse(process.argv[2]);
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const root=await fs.realpath(args.directory);
const seconds=args.seconds??3;
const size=args.viewport_size??[1280,720];
if(!Number.isFinite(seconds)||seconds<0||seconds>60||size.length!==2||size.some(n=>!Number.isInteger(n)||n<64||n>4096))throw Error('Invalid duration or viewport_size');
if(typeof WebSocket==='undefined')throw Error('web_playtest requires Node 22+');
const profile=await fs.mkdtemp(path.join(os.tmpdir(),'engine-mcp-browser-'));
const mime={'.html':'text/html','.js':'text/javascript','.wasm':'application/wasm','.json':'application/json','.png':'image/png','.wav':'audio/wav','.ogg':'audio/ogg'};
let browser,ws,server;
try{
  server=http.createServer(async(req,res)=>{
    try{
      const pathname=decodeURIComponent(new URL(req.url,'http://localhost').pathname);
      const file=await fs.realpath(path.join(root,pathname==='/'?'index.html':pathname));
      if(file!==root&&!file.startsWith(root+path.sep)){res.writeHead(403).end();return;}
      res.writeHead(200,{'Content-Type':mime[path.extname(file)]??'application/octet-stream'}).end(await fs.readFile(file));
    }catch{res.writeHead(404).end();}
  });
  await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve);});
  const url=`http://127.0.0.1:${server.address().port}/?example=player`;
  const candidates=args.browser?[args.browser]:process.env.ENGINE_BROWSER?[process.env.ENGINE_BROWSER]:['chromium','chromium-browser','google-chrome','brave-browser','/opt/brave-bin/brave'];
  const flags=['--headless=new','--no-sandbox','--disable-dev-shm-usage','--disable-background-networking','--enable-unsafe-webgpu','--enable-features=Vulkan','--use-angle=vulkan','--remote-debugging-port=0',`--user-data-dir=${profile}`,'about:blank'];
  let endpoint;
  for(const executable of candidates){
    const child=spawn(executable,flags,{stdio:['ignore','ignore','pipe']});
    let stderr='',spawnError;
    child.on('error',error=>{spawnError=error;});
    child.stderr.on('data',data=>{stderr=(stderr+data).slice(-20000);});
    const deadline=Date.now()+15000;
    while(Date.now()<deadline&&!spawnError&&child.exitCode===null){
      endpoint=stderr.match(/DevTools listening on (ws:\/\/[^\s]+)/)?.[1];
      if(endpoint)break;
      await sleep(50);
    }
    if(endpoint){browser=child;break;}
    child.kill();
    if(!spawnError||spawnError.code!=='ENOENT')throw Error(`Browser failed to start: ${spawnError?.message??stderr}`);
  }
  if(!browser)throw Error('No Chromium browser found; set ENGINE_BROWSER or pass browser');
  ws=new WebSocket(endpoint);
  const pending=new Map();let sequence=0,session;
  const logs=[],errors=[],networkFailures=[];
  ws.addEventListener('message',event=>{
    const message=JSON.parse(event.data);
    if(message.id){const job=pending.get(message.id);if(job){pending.delete(message.id);clearTimeout(job.timer);message.error?job.reject(Error(message.error.message)):job.resolve(message.result);}return;}
    if(message.sessionId!==session)return;
    const p=message.params;
    if(message.method==='Runtime.consoleAPICalled'){
      const text=p.args.map(a=>a.value??a.description??a.type).join(' ');
      if(logs.length<1000)logs.push({type:p.type,text});
      if(p.type==='error'&&errors.length<200)errors.push(text);
    }
    if(message.method==='Runtime.exceptionThrown'&&errors.length<200)errors.push(p.exceptionDetails.exception?.description??p.exceptionDetails.text);
    if(message.method==='Network.loadingFailed'&&networkFailures.length<200)networkFailures.push({error:p.errorText,type:p.type});
    if(message.method==='Network.responseReceived'&&p.response.status>=400&&!p.response.url.endsWith('/favicon.ico')&&networkFailures.length<200)networkFailures.push({url:p.response.url,status:p.response.status});
  });
  const call=(method,params={},page=true)=>new Promise((resolve,reject)=>{
    const id=++sequence;
    const timer=setTimeout(()=>{pending.delete(id);reject(Error(`${method} timed out`));},15000);
    pending.set(id,{resolve,reject,timer});
    ws.send(JSON.stringify({id,method,params,...page&&session?{sessionId:session}:{}}));
  });
  await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});
  const {targetId}=await call('Target.createTarget',{url:'about:blank'},false);
  ({sessionId:session}=await call('Target.attachToTarget',{targetId,flatten:true},false));
  await call('Runtime.enable');await call('Page.enable');await call('Network.enable');
  await call('Emulation.setDeviceMetricsOverride',{width:size[0],height:size[1],deviceScaleFactor:1,mobile:false});
  await call('Page.addScriptToEvaluateOnNewDocument',{source:`
    window.__engineMcp={deviceCreated:false,frames:0};
    const tick=()=>{window.__engineMcp.frames++;requestAnimationFrame(tick)};requestAnimationFrame(tick);
    if(globalThis.GPUAdapter){const original=GPUAdapter.prototype.requestDevice;GPUAdapter.prototype.requestDevice=async function(...args){const device=await original.apply(this,args);window.__engineMcp.deviceCreated=true;return device;};}
  `});
  const navigation=await call('Page.navigate',{url});
  if(navigation.errorText)throw Error(navigation.errorText);
  let state;const deadline=Date.now()+30000;
  while(Date.now()<deadline){
    const evaluated=await call('Runtime.evaluate',{expression:`(()=>{const c=document.querySelector('canvas');return {webgpu:!!navigator.gpu,deviceCreated:window.__engineMcp?.deviceCreated,frames:window.__engineMcp?.frames,canvas:c?{width:c.width,height:c.height}:null,loading:document.querySelector('#loading')?.textContent??null}})()`,returnByValue:true});
    state=evaluated.result.value;
    if(state?.deviceCreated&&state.canvas?.width>1&&state.canvas?.height>1&&state.frames>5&&!state.loading)break;
    if(errors.length)break;
    await sleep(100);
  }
  await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Shift',code:'ShiftLeft',windowsVirtualKeyCode:16});
  await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Shift',code:'ShiftLeft',windowsVirtualKeyCode:16});
  await sleep(seconds*1000);
  const screenshot=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
  if(args.save_path)await fs.writeFile(args.save_path,Buffer.from(screenshot.data,'base64'));
  const ready=!!(state?.deviceCreated&&state.canvas?.width>1&&state.canvas?.height>1&&!state.loading);
  process.stdout.write(JSON.stringify({ok:ready&&!errors.length&&!networkFailures.length,ready,state,seconds,viewport_size:size,console:logs,errors,network_failures:networkFailures,png_base64:screenshot.data}));
}finally{
  ws?.close();
  if(browser){browser.kill();const deadline=Date.now()+3000;while(browser.exitCode===null&&Date.now()<deadline)await sleep(50);if(browser.exitCode===null)browser.kill('SIGKILL');}
  if(server){server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
  await fs.rm(profile,{recursive:true,force:true});
}
