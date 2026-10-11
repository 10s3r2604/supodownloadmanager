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
  connections:1, maxParallel:1, proxy:'none',
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
    string directory = null, name = null, url = null;
    bool probe = false;
    for (int i = 0; i < args.Length; i++) {
      if (args[i] == "-P" && args[i + 1].StartsWith("home:")) directory = args[i + 1].Substring(5);
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
const server=http.createServer((req,res)=>{
  res.writeHead(200,{'Content-Type':'application/octet-stream','Content-Length':bytes.length});
  if(req.method==='HEAD') { res.end(); return; }
  let offset=0;
  const timer=setInterval(()=>{
    const end=Math.min(bytes.length,offset+64_000);
    res.write(bytes.subarray(offset,end)); offset=end;
    if(offset===bytes.length) { clearInterval(timer); res.end(); }
  },req.url==='/slow.bin'?2_000:200);
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
    const main=await until(()=>pages().find(page=>!page.url().includes('#progress/')&&page.url().includes(':1420')),'Main WebView was not found');
    await main.getByRole('button',{name:'Settings',exact:true}).waitFor({timeout:20_000});
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
        $h = (Get-Process -Id ${desktop.pid}).MainWindowHandle;
        if ($h -eq 0) { throw 'Main native window was not found' };
        $result = [UIntPtr]::Zero;
        if ([UiProbe]::SendMessageTimeout($h, 0, [UIntPtr]::Zero, [IntPtr]::Zero, 2, 1000, [ref]$result) -eq [IntPtr]::Zero) { throw 'Native Windows message loop stopped responding' }
      `],{timeout:10_000,stdio:'pipe'});
    }
    async function prompt(pageUrl) {
      await invoke('plugin:event|emit',{event:'sdm:media-prompt',payload:{pageUrl,title:'Home / X'}});
    }

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
    fs.writeFileSync(path.join(output,'results.json'),JSON.stringify({passed:report.length,report},null,2));
    console.log(JSON.stringify({passed:report.length,report},null,2));
  } catch(e) {
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
