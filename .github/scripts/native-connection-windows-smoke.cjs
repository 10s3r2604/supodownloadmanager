// Exercise the release GUI executable's actual Native Messaging stdio and pipe,
// then file capture in real Chrome/Edge with all HTTP bridge calls forbidden.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const os=require('node:os');
const http=require('node:http');
const crypto=require('node:crypto');
const {spawn,execFileSync}=require('node:child_process');
const {chromium}=require(path.join(process.env.SDM_SMOKE_NODE_MODULES,'playwright-core'));
assert.equal(process.platform,'win32');
const root=process.cwd();
const output=path.join(root,'native-connection-smoke');fs.mkdirSync(output,{recursive:true});
const data=path.join(process.env.APPDATA,'com.supodownloadmanager.app');
const temp=fs.mkdtempSync(path.join(process.env.RUNNER_TEMP||os.tmpdir(),'sdm-native-'));
// DevTools' loadUnpacked API cannot resolve this runner's Unicode directory.
// Keep the extension path ASCII and exercise Unicode in the real host path.
const hostDirectory=path.join(temp,'Ứng dụng Supo');fs.mkdirSync(hostDirectory,{recursive:true});
const exe=path.join(hostDirectory,'sdm-desktop.exe');fs.copyFileSync(path.join(root,'target/release/sdm-desktop.exe'),exe);
const extension=path.join(temp,'extension','chromium');
fs.mkdirSync(extension,{recursive:true});
fs.cpSync(path.join(root,'extension/dist/chromium'),extension,{recursive:true});
const normalized=extension[0].toUpperCase()+extension.slice(1);
const expectedId=[...crypto.createHash('sha256').update(Buffer.from(normalized,'utf16le')).digest().subarray(0,16)]
 .flatMap(byte=>[String.fromCharCode(97+(byte>>4)),String.fromCharCode(97+(byte&15))]).join('');
const origin=`chrome-extension://${expectedId}/`;
const snapshots=['settings.json','settings.json.bak','downloads.json','downloads.json.bak','queues.json','queues.json.bak']
 .map(name=>({file:path.join(data,name),bytes:fs.existsSync(path.join(data,name))?fs.readFileSync(path.join(data,name)):null}));
const downloads=path.join(temp,'Tải xuống');fs.mkdirSync(downloads,{recursive:true});
const version=JSON.parse(fs.readFileSync('apps/desktop/src-tauri/tauri.conf.json','utf8')).version;
const report=[],children=[],contexts=[],requests=[];
let stage='setup';
function checkpoint(name){stage=name;console.log(JSON.stringify({stage:name}));}
function passed(value){report.push(value);fs.writeFileSync(path.join(output,'results.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(value));}
const watchdog=setTimeout(()=>{console.error(`Native test exceeded four minutes at ${stage}`);for(const pid of children)stop(pid);process.exit(1)},240000);watchdog.unref();
const bytes=Buffer.alloc(128*1024,0x53);
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function until(fn,description,timeout=30000){
 const end=Date.now()+timeout;let error;
 while(Date.now()<end){try{const value=await fn();if(value)return value}catch(e){error=e}await sleep(150)}
 throw new Error(`${description}${error?`: ${error.message}`:''}`);
}
function native(message){return new Promise((resolve,reject)=>{
 const child=spawn(exe,[origin],{windowsHide:true,stdio:['pipe','pipe','pipe']});
 let result=Buffer.alloc(0),stderr='',settled=false;
 const timer=setTimeout(()=>{child.kill();reject(new Error(`Native host did not return a framed reply at ${stage}; received ${result.length} bytes`))},10000);
 child.stdout.on('data',chunk=>{
  result=Buffer.concat([result,chunk]);
  if(!settled&&result.length>=4&&result.length>=result.readUInt32LE(0)+4){
   settled=true;clearTimeout(timer);
   try{assert.equal(result.readUInt32LE(0),result.length-4);resolve(JSON.parse(result.subarray(4).toString('utf8')))}catch(error){reject(error)}
  }
 });child.stderr.on('data',chunk=>{stderr+=chunk});
 child.on('error',error=>{clearTimeout(timer);reject(error)});
 child.on('close',code=>{clearTimeout(timer);if(settled)return;try{
  assert.equal(code,0,stderr);assert.fail(`Native host exited without a complete reply at ${stage}`);
 }catch(error){reject(error)}});
 const body=Buffer.from(JSON.stringify(message));const size=Buffer.alloc(4);size.writeUInt32LE(body.length);child.stdin.end(Buffer.concat([size,body]));
})}
const ping=()=>native({action:'request',path:'/api/ping',browser:'chrome',headers:{},timeoutMs:2500});
function stop(pid){try{execFileSync('taskkill',['/PID',String(pid),'/T','/F'],{stdio:'ignore',windowsHide:true})}catch{}}
const server=http.createServer((req,res)=>{
 if(req.url==='/page'){
  res.setHeader('Content-Type','text/html');res.end('<!doctype html><a href="/native-chrome.exe">Chrome EXE</a><a href="/native-edge.zip">Edge ZIP</a>');return;
 }
 requests.push({method:req.method,url:req.url,cookie:req.headers.cookie,referer:req.headers.referer,range:req.headers.range});
 const match=/bytes=(\d+)-(\d*)/.exec(req.headers.range||'');
 const start=match?Number(match[1]):0,end=match&&match[2]?Number(match[2]):bytes.length-1;
 const payload=bytes.subarray(start,end+1);
 res.writeHead(match?206:200,{'Content-Type':'application/octet-stream','Content-Disposition':`attachment; filename="${req.url.slice(1)}"`,
  'Content-Length':payload.length,'Accept-Ranges':'bytes',...(match?{'Content-Range':`bytes ${start}-${end}/${bytes.length}`}:{})});
 if(req.method==='HEAD'){res.end();return}
 res.write(payload.subarray(0,64));const timer=setTimeout(()=>res.end(payload.subarray(64)),1200);
 res.on('close',()=>clearTimeout(timer));
});
(async()=>{
 try{
  fs.mkdirSync(data,{recursive:true});
  for(const snapshot of snapshots)fs.rmSync(snapshot.file,{force:true});
  fs.writeFileSync(path.join(data,'settings.json'),JSON.stringify({downloadDir:downloads,categoryFolders:false,language:'en',
   browserIntegration:true,browserSetupCompleted:true,confirmBrowserDownloads:false,progressWindow:false,completeDialog:false,
   clipboardMonitor:false,notifications:false,autoUpdate:false,closeToTray:false,maxParallel:3,connections:1,proxy:'none'}));
  checkpoint('register-release-host');execFileSync(exe,['--register-browser-connection',expectedId],{windowsHide:true,timeout:10000});
  const desktop=spawn(exe,['--autostart'],{windowsHide:true,stdio:'ignore'});children.push(desktop.pid);
  checkpoint('release-host-ping');await until(async()=>{const result=await ping();return result.status===200&&JSON.parse(result.body).version===version},'Release app did not start its native pipe');
  passed({name:'release-gui-stdio-and-native-pipe',passed:true,version});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const site=`http://127.0.0.1:${server.address().port}`;
  for(const [browser,relative,file]of [['chrome','Google/Chrome/Application/chrome.exe','native-chrome.exe'],['edge','Microsoft/Edge/Application/msedge.exe','native-edge.zip']]){
   checkpoint(`${browser}-load-extension`);const executablePath=[process.env.ProgramFiles,process.env['ProgramFiles(x86)']].filter(Boolean).map(base=>path.join(base,relative)).find(p=>fs.existsSync(p));
   assert(executablePath,`${browser} is required`);
   const context=await chromium.launchPersistentContext(path.join(temp,`${browser}-profile`),{executablePath,headless:true,ignoreDefaultArgs:['--disable-extensions'],
    args:['--no-sandbox','--lang=en-US','--enable-unsafe-extension-debugging']});contexts.push(context);
   const cdp=await context.browser().newBrowserCDPSession();await cdp.send('Browser.setDownloadBehavior',{behavior:'default'});
   const installed=await cdp.send('Extensions.loadUnpacked',{path:extension});assert.equal(installed.id,expectedId);
   const worker=await until(()=>context.serviceWorkers().find(w=>w.url().startsWith(origin)),`${browser} worker did not start`);
   await worker.evaluate(()=>{globalThis.fetch=()=>{throw new Error('HTTP bridge calls are forbidden in this fixture')};return chrome.storage.local.set({port:65500})});
   const popup=await context.newPage();await popup.goto(`${origin}popup.html`);
   checkpoint(`${browser}-native-status`);const status=await popup.evaluate(()=>chrome.runtime.sendMessage({type:'connection-status'}));
   assert.equal(status.transport,'native');assert.equal(status.status.version,version);
   assert.equal(await popup.locator('#port').isVisible(),false);
   await popup.screenshot({path:path.join(output,`${browser}-connected.png`)});
   if(browser==='chrome'){
    checkpoint('native-video-probe');const clientId='native-quality-fixture';
    let probe=await popup.evaluate(clientId=>chrome.runtime.sendMessage({type:'video-probe',clientId,pageUrl:'https://fixture.example/native-video'}),clientId);
    assert(probe.ok,JSON.stringify(probe));
    probe=await until(async()=>{const next=await popup.evaluate(clientId=>chrome.runtime.sendMessage({type:'video-probe-status',clientId}),clientId);assert(next.ok,JSON.stringify(next));return next.info?next:null},'Native video qualities were not returned');
    assert(probe.info.options.length>0);assert(probe.info.options.some(option=>option.size>0));
    await popup.evaluate(clientId=>chrome.runtime.sendMessage({type:'video-probe-cancel',clientId}),clientId);
    passed({name:'native-video-quality-size-and-cancellation',passed:true});
   }
   checkpoint(`${browser}-capture-file`);await context.addCookies([{name:'native_session',value:'fixture',url:site}]);
   const page=await context.newPage();await page.goto(`${site}/page`);
   await page.getByRole('link',{name:browser==='chrome'?'Chrome EXE':'Edge ZIP',exact:true}).click();
   await until(()=>{
    const history=JSON.parse(fs.readFileSync(path.join(data,'downloads.json'),'utf8'));
    const rows=Array.isArray(history)?history:history.downloads;
    return rows?.find(row=>row.fileName===file&&row.state==='completed');
   },`${browser} download did not complete in SDM`);
   assert.deepEqual(fs.readFileSync(path.join(downloads,file)),bytes);
   assert(requests.some(request=>request.url===`/${file}`&&request.method==='GET'&&request.cookie==='native_session=fixture'&&request.referer===`${site}/page`));
   const browserDownloads=await popup.evaluate(()=>chrome.downloads.search({}));assert(!browserDownloads.some(item=>item.filename?.endsWith(file)&&item.state==='complete'));
   passed({name:`${browser}-native-file-capture-with-cookies-and-referrer`,passed:true,bytes:bytes.length});
   await context.close();contexts.splice(contexts.indexOf(context),1);
  }
  checkpoint('closed-app-status');stop(children.pop());await until(async()=> (await ping()).error==='appNotRunning','A closed app must return appNotRunning');
  checkpoint('explicit-launch');const launched=await native({action:'launch'});assert(launched.launched&&launched.pid);children.push(launched.pid);
  await until(async()=> (await ping()).status===200,'Native Open SDM did not reconnect');
  passed({name:'closed-app-and-explicit-launch-reconnect',passed:true});
  fs.writeFileSync(path.join(output,'results.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
 }catch(error){console.error(`Failure at ${stage}: ${error.stack||error}`);throw error}finally{
  for(const context of contexts)await context.close().catch(()=>{});for(const pid of children)stop(pid);
  for(const snapshot of snapshots){if(snapshot.bytes)fs.writeFileSync(snapshot.file,snapshot.bytes);else fs.rmSync(snapshot.file,{force:true})}
  server.closeAllConnections();server.close();clearTimeout(watchdog);
 }
})().catch(error=>{console.error(error);process.exitCode=1});
