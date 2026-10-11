// Exercise native Windows IPC and WebView2 window creation, not the mock backend.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const crypto = require('node:crypto');
const {spawn, execFileSync} = require('node:child_process');
const {chromium} = require(path.join(process.env.SDM_SMOKE_NODE_MODULES, 'playwright-core'));

if (process.platform !== 'win32') throw new Error('This smoke test requires Windows/WebView2.');
const root = process.cwd();
const output = path.join(root, 'progress-smoke');
const directory = path.join(process.env.RUNNER_TEMP, 'sdm-progress-downloads');
fs.mkdirSync(output, {recursive:true});
fs.mkdirSync(directory, {recursive:true});
const data = path.join(process.env.APPDATA, 'com.supodownloadmanager.app');
fs.mkdirSync(data, {recursive:true});
fs.writeFileSync(path.join(data, 'settings.json'), JSON.stringify({
  downloadDir:directory, language:'en', progressWindow:true, progressStyle:'graph',
  completeDialog:true, clipboardMonitor:false, notifications:false, closeToTray:false,
  connections:1, maxParallel:1, proxy:'none', confirmBrowserDownloads:true,
}));
// A deterministic subprocess fixture writes media bytes and progress lines.
// This tests the native media manager without relying on an external site.
const tools=path.join(data,'tools');
fs.mkdirSync(tools,{recursive:true});
const fixture=path.join(output,'MediaFixture.cs');
const probeInfo=JSON.stringify({title:'Media progress smoke',duration:10,extractor_key:'Twitter',
  formats:[{format_id:'18',ext:'mp4',height:720,fps:30,vcodec:'avc1.64001f',acodec:'mp4a.40.2',filesize:27}],
}).replaceAll('"','""');
fs.writeFileSync(fixture,`
using System;
using System.IO;
using System.Text;
using System.Threading;
class MediaFixture {
  static int Main(string[] args) {
    Console.OutputEncoding = new UTF8Encoding(false);
    string directory = null, temporary = null, name = null, url = null;
    bool probe = false;
    for (int i = 0; i < args.Length; i++) {
      if (args[i] == "-P" && args[i + 1].StartsWith("home:")) directory = args[i + 1].Substring(5);
      if (args[i] == "-P" && args[i + 1].StartsWith("temp:")) temporary = args[i + 1].Substring(5);
      if (args[i] == "-o") name = args[i + 1].Replace("%(ext)s", "mp4").Replace("%%", "%");
      if (args[i] == "--") url = args[i + 1];
      if (args[i] == "-J") probe = true;
    }
    if (probe) {
      string marker = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "probe.pid");
      if (url.Contains("/status/987") && !File.Exists(marker)) {
        File.WriteAllText(marker, System.Diagnostics.Process.GetCurrentProcess().Id.ToString());
        Console.Error.WriteLine("[twitter] Reading stalled post");
        Thread.Sleep(30000);
      }
      Console.Error.WriteLine("[twitter] Reading video formats");
      Thread.Sleep(1500);
      Console.WriteLine(@"${probeInfo}");
      return 0;
    }
    if (url != null && url.Contains("broken")) {
      Console.Error.WriteLine("ERROR: media smoke fixture failure");
      return 1;
    }
    Console.WriteLine("SDMP downloading 5 27 NA 10 18");
    Directory.CreateDirectory(temporary);
    File.WriteAllText(Path.Combine(temporary, "fixture.part"), "partial-media");
    Thread.Sleep(1000);
    Directory.CreateDirectory(directory);
    string file = Path.Combine(directory, name);
    File.WriteAllText(file, "native-media-progress-smoke", new UTF8Encoding(false));
    Console.WriteLine("SDMP finished 27 27 NA 10 18");
    Console.WriteLine("SDMF " + file);
    return 0;
  }
}
`);
execFileSync(path.join(process.env.WINDIR,'Microsoft.NET/Framework64/v4.0.30319/csc.exe'),[
  '/nologo','/target:exe',`/out:${path.join(tools,'yt-dlp.exe')}`,fixture,
]);
fs.writeFileSync(path.join(tools,'yt-dlp.checked'),'fixture');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const children=[];
const report=[];
async function until(fn, message, timeout=20_000) {
  const end=Date.now()+timeout;
  let last;
  while(Date.now()<end) {
    try { const result=await fn(); if(result) return result; } catch(e) { last=e; }
    await sleep(200);
  }
  throw new Error(`${message}${last ? `: ${last.message}` : ''}`);
}
async function bounded(promise, message, timeout=8_000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_,reject)=>{
      timer=setTimeout(()=>reject(new Error(message)),timeout);
    })]);
  } finally { clearTimeout(timer); }
}
function child(exe, args, name, env=process.env) {
  const log=fs.openSync(path.join(output,`${name}.log`),'w');
  const processHandle=spawn(exe,args,{cwd:root,env,stdio:['ignore',log,log]});
  processHandle.on('exit',(code,signal)=>console.log(`${name} exited: ${code} ${signal||''}`));
  children.push(processHandle);
  return processHandle;
}
const bytes=Buffer.alloc(256_000,0x53);
const fileRequests=[];
const server=http.createServer((req,res)=>{
  const probe=req.headers.range==='bytes=0-0';
  fileRequests.push({url:req.url,method:req.method,headers:req.headers,probe});
  // An unresponsive one-byte probe must not block confirmation or overwrite
  // the browser's filename and size when it eventually completes.
  if(probe && req.url==='/delayed.exe') {
    setTimeout(()=>res.writeHead(206,{'Content-Length':1,'Content-Range':`bytes 0-0/${bytes.length}`}).end(bytes.subarray(0,1)),3_000);
    return;
  }
  const range=/^bytes=(\d+)-(\d*)$/.exec(req.headers.range||'');
  const start=range?Number(range[1]):0;
  const end=range&&range[2]?Math.min(Number(range[2])+1,bytes.length):bytes.length;
  res.writeHead(range?206:200,{'Content-Type':'application/octet-stream','Content-Length':end-start,
    ...(range?{'Content-Range':`bytes ${start}-${end-1}/${bytes.length}`}:{})});
  if(req.method==='HEAD') { res.end(); return; }
  let offset=start;
  const timer=setInterval(()=>{
    const next=Math.min(end,offset+64_000);
    res.write(bytes.subarray(offset,next)); offset=next;
    if(offset===end) { clearInterval(timer); res.end(); }
  },req.url==='/slow.bin'||req.url.startsWith('/temp-')?2_000:200);
  res.on('close',()=>clearInterval(timer));
});

(async()=>{
  let browser;
  try {
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    const source=`http://127.0.0.1:${server.address().port}`;
    child(process.execPath,[path.join(root,'apps/desktop/node_modules/vite/bin/vite.js'),path.join(root,'apps/desktop'),'--host','127.0.0.1','--port','1420'],'vite');
    await until(async()=>{
      const response=await fetch('http://127.0.0.1:1420/',{signal:AbortSignal.timeout(1_000)});
      return response.ok;
    },'Vite did not start');
    const desktop=child(path.join(root,'target/debug/sdm-desktop.exe'),[],'desktop',{
      ...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9222',
    });
    await until(async()=>{
      const response=await fetch('http://127.0.0.1:9222/json/version',{signal:AbortSignal.timeout(1_000)});
      return response.ok;
    },'WebView2 debugging endpoint did not start',30_000);
    browser=await chromium.connectOverCDP('http://127.0.0.1:9222');
    const pages=()=>browser.contexts().flatMap(context=>context.pages());
    const main=await until(()=>pages().find(page=>!page.url().includes('#')&&page.url().includes(':1420')),'Main WebView was not found');
    await main.getByRole('button',{name:'Settings',exact:true}).waitFor({timeout:20_000});
    // Remember the actual main HWND before opening auxiliary windows.
    // Process.MainWindowHandle can later select one of the small windows.
    const mainWindowHandle=execFileSync('powershell.exe',['-NoProfile','-Command',
      `(Get-Process -Id ${desktop.pid}).MainWindowHandle.ToInt64()`
    ],{encoding:'utf8',timeout:10_000}).trim();
    assert(/^\d+$/.test(mainWindowHandle) && Number(mainWindowHandle)>0);
    async function invoke(command,args={}) {
      return bounded(main.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args}),`${command} blocked the Windows UI thread`);
    }
    async function progress(id,name) {
      const page=await until(()=>pages().find(page=>page.url().endsWith(`#progress/${id}`)),`${name}: progress WebView was not created`);
      await page.locator('.pw-graph').waitFor({timeout:10_000});
      assert((await page.locator('.pw-name').innerText()).length>0);
      await page.screenshot({path:path.join(output,`${name}.png`),timeout:5_000});
      assert(await main.getByRole('button',{name:'Settings',exact:true}).isVisible());
      report.push({name,id,rendered:true,mainResponsive:true});
      return page;
    }
    async function closeProgress(page) {
      await Promise.all([
        page.waitForEvent('close',{timeout:8_000}),
        page.getByRole('button',{name:'Close',exact:true}).click(),
      ]);
    }

    function nativeResponsive() {
      // WM_NULL reaches the actual Windows message loop. JavaScript alone
      // can keep responding while a synchronous native IPC handler is stuck.
      execFileSync('powershell.exe',['-NoProfile','-Command',`
        Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class UiProbe { [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr h, uint m, UIntPtr w, IntPtr l, uint f, uint t, out UIntPtr result); }';
        $h = [IntPtr]${mainWindowHandle};
        if ($h -eq 0) { throw 'Main native window was not found' };
        $result = [UIntPtr]::Zero;
        if ([UiProbe]::SendMessageTimeout($h, 0, [UIntPtr]::Zero, [IntPtr]::Zero, 2, 1000, [ref]$result) -eq [IntPtr]::Zero) { throw 'Native Windows message loop stopped responding' }
      `],{timeout:10_000,stdio:'pipe'});
    }
    async function prompt(pageUrl) {
      await invoke('plugin:event|emit',{event:'sdm:media-prompt',payload:{pageUrl,title:'Home / X'}});
    }

    async function capture(name, route='/delayed.exe') {
      const response=await fetch('http://127.0.0.1:17360/api/download',{
        method:'POST',headers:{'Content-Type':'application/json','X-SDM-Client':'extension','X-SDM-Browser':'chrome'},
        body:JSON.stringify({url:`${source}${route}`,fileName:name,fileSize:bytes.length,
          cookies:'download_session=fixture',referrer:'https://github.com/example/releases',userAgent:'SDM-native-browser-fixture'}),
      });
      assert.equal(response.status,202);
      const page=await until(async()=>{
        for(const candidate of pages().filter(p=>p.url().includes('#add/'))) {
          if(await candidate.getByRole('textbox',{name:'File name',exact:true}).inputValue().catch(()=>null)===name) return candidate;
        }
      },`Browser confirmation for ${name} did not open`);
      await page.getByRole('button',{name:'Download now',exact:true}).waitFor();
      assert(await invoke('plugin:window|is_minimized',{label:'main'}),'Capture restored the main SDM window');
      assert.equal(await main.getByRole('dialog').count(),0,'Capture opened a modal in the main window');
      const rectangle=await page.getByRole('dialog').boundingBox();
      assert(rectangle.width<=560 && rectangle.height<=400,`Confirmation is too large: ${JSON.stringify(rectangle)}`);
      await page.getByText('250 KB',{exact:true}).waitFor();
      assert(!(await invoke('list_downloads')).some(d=>d.fileName===name),'Capture started without confirmation');
      return page;
    }
    async function closeAdd(page, action) {
      await Promise.all([page.waitForEvent('close',{timeout:8_000}),action()]);
      assert(await invoke('plugin:window|is_minimized',{label:'main'}));
      await invoke('list_downloads'); // Closing a confirmation must not quit SDM.
    }
    await invoke('plugin:window|minimize',{label:'main'});
    await until(()=>invoke('plugin:window|is_minimized',{label:'main'}),'Main SDM window did not minimize');
    await assert.rejects(invoke('get_add_prompt'),/no browser download/);
    const cancelled=await capture('browser-installer.exe');
    await cancelled.screenshot({path:path.join(output,'browser-confirmation.png')});
    const cancelledPrompt=await cancelled.evaluate(()=>window.__TAURI_INTERNALS__.invoke('get_add_prompt'));
    assert.equal(cancelledPrompt.fileName,'browser-installer.exe');
    assert(cancelledPrompt.directory.endsWith('Programs'));
    assert(cancelledPrompt.headers.some(([key,value])=>key==='Cookie' && value==='download_session=fixture'));
    await sleep(3_500);
    assert.equal(await cancelled.getByRole('textbox',{name:'File name',exact:true}).inputValue(),'browser-installer.exe');
    nativeResponsive();
    await closeAdd(cancelled,()=>cancelled.getByRole('button',{name:'Cancel',exact:true}).click());
    assert(!fileRequests.some(r=>r.method==='GET' && !r.probe && r.url==='/delayed.exe'));
    report.push({name:'browser-small-independent-confirmation',passed:true,mainMinimized:true,filenamePreserved:true});

    const first=await capture('browser-first.zip','/first.zip');
    const second=await capture('browser-second.exe','/second.exe');
    assert.equal(await first.getByRole('textbox',{name:'File name',exact:true}).inputValue(),'browser-first.zip');
    await closeAdd(second,()=>second.keyboard.press('Escape'));
    assert(!first.isClosed());
    await closeAdd(first,()=>first.getByRole('button',{name:'Close',exact:true}).click());
    report.push({name:'browser-multiple-cancel-escape',passed:true});

    const later=await capture('browser-later.zip','/later.zip');
    // At the smallest supported size the fields scroll but all actions fit.
    await later.evaluate(()=>window.__TAURI_INTERNALS__.invoke('plugin:window|set_size',{
      label:window.__TAURI_INTERNALS__.metadata.currentWindow.label,
      value:{Logical:{width:320,height:240}},
    }));
    await until(async()=>await later.evaluate(()=>window.innerWidth<=320),'Confirmation did not resize');
    for(const name of ['Cancel','Download later','Download now']) {
      const button=later.getByRole('button',{name,exact:true});
      const fits=await button.evaluate(el=>{const r=el.getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight&&r.left>=0&&r.right<=innerWidth;});
      assert(fits,`${name} is clipped in the small window`);
    }
    await later.screenshot({path:path.join(output,'browser-confirmation-small.png')});
    await closeAdd(later,()=>later.getByRole('button',{name:'Download later',exact:true}).click());
    const paused=(await invoke('list_downloads')).find(d=>d.fileName==='browser-later.zip');
    assert(paused && paused.state==='paused');
    assert(!fileRequests.some(r=>r.method==='GET' && !r.probe && r.url==='/later.zip'));
    await invoke('remove_downloads',{ids:[paused.id],deleteFiles:false});
    report.push({name:'browser-download-later-small-window',passed:true});

    const now=await capture('browser-now.zip','/now.zip');
    const custom=path.join(directory,'browser-custom');
    await now.getByRole('textbox',{name:'File name',exact:true}).fill('renamed.zip');
    await now.locator('.input-row input').fill(custom);
    await closeAdd(now,()=>now.getByRole('button',{name:'Download now',exact:true}).click());
    const downloaded=await until(async()=>{
      const d=(await invoke('list_downloads')).find(d=>d.fileName==='renamed.zip');
      return d?.state==='completed'?d:null;
    },'Confirmed browser file did not complete');
    assert.equal(path.dirname(downloaded.path),custom);
    assert.equal(crypto.createHash('sha256').update(fs.readFileSync(downloaded.path)).digest('hex'),crypto.createHash('sha256').update(bytes).digest('hex'));
    const request=fileRequests.find(r=>r.method==='GET' && !r.probe && r.url==='/now.zip');
    assert.equal(request.headers.cookie,'download_session=fixture');
    assert.equal(request.headers.referer,'https://github.com/example/releases');
    assert.equal(request.headers['user-agent'],'SDM-native-browser-fixture');
    await closeProgress(await progress(downloaded.id,'browser-confirmed-download'));
    report.push({name:'browser-confirmed-download-content-and-auth',passed:true});
    console.log(JSON.stringify({browserConfirmationChecks:report},null,2));
    // Restore through a native user action after testing that capture leaves
    // it minimized; these checks never grant an extra application permission.
    execFileSync('powershell.exe',['-NoProfile','-Command',`
      Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class RestoreSdm { [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int command); }';
      [RestoreSdm]::ShowWindowAsync([IntPtr]${mainWindowHandle},9) | Out-Null;
    `],{timeout:10_000,stdio:'pipe'});
    await until(async()=>!await invoke('plugin:window|is_minimized',{label:'main'}),'Main window did not restore');

    // The old extension sent Home rather than the post. Fail promptly with
    // an actionable message, without starting an extractor for the feed.
    await prompt('https://x.com/home');
    await main.getByText('Open the X post containing the video',{exact:false}).waitFor({timeout:8_000});
    nativeResponsive();
    await main.getByRole('button',{name:'Cancel',exact:true}).click();
    report.push({name:'x-home-error',passed:true,nativeResponsive:true});

    // Real native media_probe runs a deliberately stalled subprocess. Check
    // status IPC, the Windows message loop, Cancel, process exit, and retry.
    await prompt('https://x.com/author/status/987');
    const pidFile=path.join(tools,'probe.pid');
    await until(()=>fs.existsSync(pidFile),'Stalled probe did not start');
    await until(async()=>Boolean(await invoke('media_probe_stage',{pageUrl:'https://x.com/author/status/987'})),'Probe stage was not readable');
    await invoke('list_downloads');
    await invoke('get_settings');
    nativeResponsive();
    await main.screenshot({path:path.join(output,'0-stalled-probe.png')});
    await main.getByRole('button',{name:'Cancel',exact:true}).click();
    await main.getByRole('dialog').waitFor({state:'hidden',timeout:3_000});
    const probePid=Number(fs.readFileSync(pidFile,'utf8'));
    assert(Number.isSafeInteger(probePid)&&probePid>0);
    await until(()=>{
      try {
        execFileSync('powershell.exe',['-NoProfile','-Command',`if (Get-Process -Id ${probePid} -ErrorAction SilentlyContinue) { exit 1 }`],{timeout:3_000,stdio:'pipe'});
        return true;
      } catch { return false; }
    },'Cancel left the extractor running',8_000);
    report.push({name:'stalled-probe-cancel',passed:true,nativeResponsive:true,childStopped:true});
    await prompt('https://x.com/author/status/987');
    await main.locator('.media-option').first().waitFor({timeout:8_000});
    nativeResponsive();
    await main.getByRole('button',{name:'Cancel',exact:true}).click();
    report.push({name:'probe-retry-after-cancel',passed:true,nativeResponsive:true});

    // Discovery and Download now both use native IPC; only yt-dlp's external
    // service is replaced by the deterministic executable fixture.
    await prompt('https://x.com/author/status/123');
    await main.locator('.media-option').first().waitFor({timeout:10_000});
    await main.getByRole('button',{name:'Download now',exact:true}).click();
    await main.getByRole('dialog').waitFor({state:'hidden',timeout:10_000});
    const media=(await invoke('list_downloads')).find(d=>d.fileName==='Media progress smoke.mp4');
    assert(media&&media.media);
    let window=await progress(media.id,'1-media-download-now');
    const mediaFile=await until(async()=>{
      const d=await invoke('get_download',{id:media.id}); return d?.state==='completed'?d:null;
    },'Native media did not complete');
    assert.equal(fs.readFileSync(mediaFile.path,'utf8'),'native-media-progress-smoke');
    const mediaStored=JSON.parse(fs.readFileSync(path.join(data,'downloads.json'),'utf8')).find(d=>d.id===media.id);
    assert(mediaStored.tempDir);
    assert(!fs.readdirSync(mediaStored.tempDir).some(name=>name.startsWith('download-')),
      'Successful media left its temporary workspace behind');
    report.push({name:'native-media-content',passed:true});
    await closeProgress(window);

    const mediaRequest={source:{pageUrl:'https://example.invalid/broken'},option:{id:'720',kind:'video',height:720,fps:30,ext:'mp4',vcodec:'H.264',acodec:'AAC',size:1_234,needsFfmpeg:false,format:'18',merge:null,audio:null},title:'Failed media',directory,start:true};
    const failedId=await invoke('add_media',{request:mediaRequest});
    window=await progress(failedId,'2-media-failure');
    await until(async()=>((await invoke('get_download',{id:failedId}))?.state==='failed'),'Expected the fixture failure');
    await window.getByText('media smoke fixture failure',{exact:false}).waitFor();
    await closeProgress(window);
    await invoke('resume_downloads',{ids:[failedId]});
    window=await progress(failedId,'3-media-resume');
    await until(async()=>((await invoke('get_download',{id:failedId}))?.state==='failed'),'Resumed media did not settle');
    await closeProgress(window);
    await invoke('resume_all');
    window=await progress(failedId,'4-resume-all');
    await until(async()=>((await invoke('get_download',{id:failedId}))?.state==='failed'),'Resume all did not settle');
    await closeProgress(window);

    const fileId=await invoke('add_download',{request:{url:`${source}/sample.bin`,fileName:'sample.bin',directory,start:true}});
    await progress(fileId,'5-file-download');
    const file=await until(async()=>{
      const d=await invoke('get_download',{id:fileId}); return d?.state==='completed'?d:null;
    },'The local file download did not complete');
    const sha=buffer=>crypto.createHash('sha256').update(buffer).digest('hex');
    assert.equal(sha(fs.readFileSync(file.path)),sha(bytes));
    report.push({name:'local-file-content',sha256:sha(bytes),passed:true});

    // Raising maxParallel starts a queued job from save_settings, another
    // command that must not create a WebView from the Windows UI thread.
    await invoke('add_download',{request:{url:`${source}/slow.bin`,fileName:'slow.bin',directory,start:true}});
    const queuedId=await invoke('add_media',{request:{source:{pageUrl:'https://example.invalid/queued'},option:{id:'720',kind:'video',height:720,fps:30,ext:'mp4',vcodec:'H.264',acodec:'AAC',size:1_234,needsFfmpeg:false,format:'18',merge:null,audio:null},title:'Queued media',directory,start:true}});
    assert.equal((await invoke('get_download',{id:queuedId})).state,'queued');
    const settings=await invoke('get_settings');
    await invoke('save_settings',{settings:{...settings,maxParallel:2}});
    await progress(queuedId,'6-settings-start-queued');

    // Configure the actual General settings form and exercise both the saved
    // root and per-download roots through real native IPC and filesystem I/O.
    const defaults=await invoke('get_settings');
    assert.equal(defaults.tempDir,path.join(process.env.LOCALAPPDATA,'com.supodownloadmanager.app','temp'));
    await invoke('save_settings',{settings:{...defaults,progressWindow:false,completeDialog:false}});
    async function setTempFolder(folder,screenshot) {
      await main.getByRole('button',{name:'Settings',exact:true}).click();
      const dialog=main.getByRole('dialog',{name:'Settings',exact:true});
      await dialog.getByRole('tab',{name:'General',exact:true}).click();
      await dialog.getByRole('textbox',{name:'Temporary folder',exact:true}).fill(folder);
      if(screenshot) await main.screenshot({path:path.join(output,screenshot)});
      await dialog.getByRole('button',{name:'Save',exact:true}).click();
      await until(async()=>!(await dialog.count()),'Settings did not save or close');
      assert.equal((await invoke('get_settings')).tempDir,folder);
      assert.equal(JSON.parse(fs.readFileSync(path.join(data,'settings.json'),'utf8')).tempDir,folder);
    }
    const tempA=path.join(process.env.LOCALAPPDATA,'sdm-smoke-temp-a');
    const tempB=path.join(process.env.RUNNER_TEMP,'sdm-smoke-temp-b');
    await setTempFolder(tempA,'temporary-folder-settings.png');
    report.push({name:'settings-custom-temp-folder',passed:true,persisted:true});
    const valid=await invoke('get_settings');
    await assert.rejects(invoke('save_settings',{settings:{...valid,tempDir:'relative-temp'}}),/absolute path/i);
    const blocked=path.join(tempA,'not-a-folder');
    fs.writeFileSync(blocked,'keep');
    await assert.rejects(invoke('save_settings',{settings:{...valid,tempDir:blocked}}));
    assert.equal((await invoke('get_settings')).tempDir,tempA);
    assert.equal(fs.readFileSync(blocked,'utf8'),'keep');
    report.push({name:'invalid-temp-settings-keep-previous-folder',passed:true,mainResponsive:true});
    function workspaces(root) {
      return fs.existsSync(root)?fs.readdirSync(root).filter(name=>name.startsWith('download-')):[];
    }
    const partialId=await invoke('add_download',{request:{url:`${source}/temp-paused.bin`,fileName:'temp-paused.bin',directory,start:true}});
    await until(async()=>{const d=await invoke('get_download',{id:partialId});return d.downloaded>0;},'Temp fixture did not transfer');
    await invoke('pause_downloads',{ids:[partialId]});
    const partial=await until(async()=>{const d=await invoke('get_download',{id:partialId});return d.state==='paused'?d:null;},'Temp fixture did not pause');
    assert(!fs.existsSync(`${partial.path}.sdm-part`));
    assert(!fs.existsSync(`${partial.path}.sdm-state`));
    assert.equal(workspaces(tempA).length,1);
    const workspaceA=path.join(tempA,workspaces(tempA)[0]);
    assert(fs.existsSync(path.join(workspaceA,'temp-paused.bin.sdm-part')));
    assert(fs.existsSync(path.join(workspaceA,'temp-paused.bin.sdm-state')));
    const stored=JSON.parse(fs.readFileSync(path.join(data,'downloads.json'),'utf8')).find(d=>d.id===partialId);
    assert.equal(stored.tempDir,tempA);
    await setTempFolder(tempB);
    assert(fs.existsSync(workspaceA),'Changing the root deleted paused data');
    await invoke('resume_downloads',{ids:[partialId]});
    const resumed=await until(async()=>{const d=await invoke('get_download',{id:partialId});return d.state==='completed'?d:null;},'Paused data did not resume after temp change');
    assert.equal(sha(fs.readFileSync(resumed.path)),sha(bytes));
    assert(!fs.existsSync(workspaceA));
    assert(!fs.existsSync(path.join(directory,'.sdm-finalizing')));
    assert.equal(fs.readFileSync(blocked,'utf8'),'keep');
    report.push({name:'temp-root-change-resume-and-completion-cleanup',passed:true,contentVerified:true});
    const removeId=await invoke('add_download',{request:{url:`${source}/temp-remove.bin`,fileName:'temp-remove.bin',directory,start:true}});
    await until(async()=>{const d=await invoke('get_download',{id:removeId});return d.downloaded>0;},'Removal fixture did not transfer');
    assert.equal(workspaces(tempB).length,1);
    await invoke('remove_downloads',{ids:[removeId],deleteFiles:false});
    await until(async()=>!(await invoke('get_download',{id:removeId})),'Removed download remained in the list');
    assert.equal(workspaces(tempB).length,0);
    assert(!fs.existsSync(path.join(directory,'temp-remove.bin')));
    report.push({name:'new-temp-root-removal-cleans-unfinished-data',passed:true});
    fs.writeFileSync(path.join(output,'results.json'),JSON.stringify({passed:report.length,report},null,2));
    console.log(JSON.stringify({passed:report.length,report},null,2));
  } catch(e) {
    console.error(`::error::Native Windows smoke: ${String(e.message).replaceAll('\n',' ')}`);
    fs.writeFileSync(path.join(output,'failure.txt'),e.stack||String(e));
    for(const name of ['desktop','vite']) {
      const log=path.join(output,`${name}.log`);
      if(fs.existsSync(log)) console.error(`${name} log:\n${fs.readFileSync(log,'utf8')}`);
    }
    try {
      console.error(execFileSync('powershell.exe',['-NoProfile','-Command',
        "Get-CimInstance Win32_Process | Where-Object { $_.Name -in @('sdm-desktop.exe', 'msedgewebview2.exe') } | Select-Object Name, CommandLine | ConvertTo-Json -Depth 5"
      ],{encoding:'utf8',timeout:5_000}));
    } catch {}
    if(browser) for(const [index,page] of browser.contexts().flatMap(c=>c.pages()).entries()) {
      await page.screenshot({path:path.join(output,`failure-${index}.png`),timeout:3_000}).catch(()=>{});
    }
    throw e;
  } finally {
    if(browser) await bounded(browser.close(),'Could not close the smoke browser',3_000).catch(()=>{});
    for(const processHandle of children.reverse()) {
      if(processHandle.pid) try { execFileSync('taskkill',['/pid',String(processHandle.pid),'/t','/f'],{stdio:'ignore'}); } catch {}
    }
    server.closeAllConnections(); server.close();
  }
})().catch(e=>{console.error(e);process.exit(1)});
