// Exercise native Windows IPC and WebView2 window creation, not the mock backend.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const crypto = require('node:crypto');
const {spawn, execFile, execFileSync} = require('node:child_process');
const {promisify} = require('node:util');
const {chromium} = require(path.join(process.env.SDM_SMOKE_NODE_MODULES, 'playwright-core'));

if (process.platform !== 'win32') throw new Error('This smoke test requires Windows/WebView2.');
const root = process.cwd();
const appVersion=JSON.parse(fs.readFileSync(path.join(root,'apps/desktop/src-tauri/tauri.conf.json'),'utf8')).version;
const output = path.join(root, 'progress-smoke');
const directory = path.join(process.env.RUNNER_TEMP, 'sdm-progress-downloads');
fs.mkdirSync(output, {recursive:true});
fs.mkdirSync(directory, {recursive:true});
const data = path.join(process.env.APPDATA, 'com.supodownloadmanager.app');
fs.mkdirSync(data, {recursive:true});
const originalSettings = ['settings.json','settings.json.bak'].map(name=>({
  file:path.join(data,name), bytes:fs.existsSync(path.join(data,name))?fs.readFileSync(path.join(data,name)):null,
}));
const fixtureSettings = {
  downloadDir:directory, language:'en', progressWindow:true, progressStyle:'graph',
  completeDialog:true, clipboardMonitor:false, notifications:false, closeToTray:false,
  connections:1, maxParallel:1, proxy:'none', confirmBrowserDownloads:true,
  startWithWindows:false, autoUpdate:false,
  browserSetupCompleted:false,
};
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
        // Long enough to remain stalled during automation host startup;
        // cancellation below still has to stop this process within 8 seconds.
        Thread.Sleep(90000);
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
const execFileAsync=promisify(execFile);
const pwsh='pwsh.exe';
const interopFile=path.join(output,'NativeInterop.ps1');
// Run Windows automation asynchronously so shell startup and native messages
// cannot stop this process from serving the HTTP fixtures. PowerShell 7 is
// supplied by windows-latest; the C# media executable above still uses csc.exe.
fs.writeFileSync(interopFile,`param([string]$Action, [uint32]$TargetProcess, [long]$WindowHandle)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class NativeSdmSmoke {
  public delegate bool Visit(IntPtr window, IntPtr state);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Visit visit, IntPtr state);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, Visit visit, IntPtr state);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr window, uint command);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window, int command);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder text, int capacity);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr window, StringBuilder text, int capacity);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, UIntPtr w, IntPtr l, uint flags, uint timeout, out UIntPtr result);
  static string Text(IntPtr window) { var text=new StringBuilder(4096); GetWindowText(window,text,text.Capacity); return text.ToString(); }
  public static long MainWindow(uint process, bool requireVisible) {
    IntPtr main=IntPtr.Zero;
    int matches=0;
    EnumWindows((window,state)=>{
      uint owner; GetWindowThreadProcessId(window,out owner);
      if(owner==process && (!requireVisible || IsWindowVisible(window)) && GetWindow(window,4)==IntPtr.Zero && Text(window)=="Supo Download Manager") {
        matches+=1; main=window;
      }
      return true;
    },IntPtr.Zero);
    if(matches>1) throw new Exception("More than one main native window was found");
    if(main==IntPtr.Zero) throw new Exception("Main native window was not found");
    return main.ToInt64();
  }
  static IntPtr MainHandle(uint process, long handle) {
    IntPtr main=new IntPtr(handle);
    uint owner; GetWindowThreadProcessId(main,out owner);
    if(!IsWindow(main) || owner!=process || Text(main)!="Supo Download Manager") throw new Exception("Cached main native window no longer belongs to this desktop process");
    return main;
  }
  static void Message(IntPtr window, uint message, UIntPtr w) {
    UIntPtr result;
    if(SendMessageTimeout(window,message,w,IntPtr.Zero,2,1000,out result)==IntPtr.Zero) throw new Exception("Native Windows message loop stopped responding");
  }
  public static void Responsive(uint process, long handle) { Message(MainHandle(process,handle),0,UIntPtr.Zero); }
  public static bool Visible(uint process, long handle) { return IsWindowVisible(MainHandle(process,handle)); }
  public static void Restore(uint process, long handle) { ShowWindowAsync(MainHandle(process,handle),9); }
  public static bool DismissRecovery(uint process) {
    IntPtr dialog=IntPtr.Zero;
    EnumWindows((window,state)=>{
      uint owner; GetWindowThreadProcessId(window,out owner);
      if(owner==process && Text(window)=="Saved data recovered") { dialog=window; return false; }
      return true;
    },IntPtr.Zero);
    if(dialog==IntPtr.Zero) return false;
    IntPtr button=IntPtr.Zero;
    EnumChildWindows(dialog,(window,state)=>{
      var kind=new StringBuilder(128); GetClassName(window,kind,kind.Capacity);
      if(kind.ToString()=="Button" && Text(window).Replace("&","").Trim()=="OK") {
        button=window; return false;
      }
      return true;
    },IntPtr.Zero);
    if(button!=IntPtr.Zero) Message(button,0x00F5,UIntPtr.Zero);
    // Windows task dialogs can host their standard button in DirectUI.
    else Message(dialog,0x0466,new UIntPtr(1));
    return true;
  }
}
'@
switch ($Action) {
  'main' { [NativeSdmSmoke]::MainWindow($TargetProcess,$true) }
  'main-any' { [NativeSdmSmoke]::MainWindow($TargetProcess,$false) }
  'visible' { [NativeSdmSmoke]::Visible($TargetProcess,$WindowHandle) }
  'responsive' { [NativeSdmSmoke]::Responsive($TargetProcess,$WindowHandle); 'responsive' }
  'restore' { [NativeSdmSmoke]::Restore($TargetProcess,$WindowHandle); 'restored' }
  'recovery' { [NativeSdmSmoke]::DismissRecovery($TargetProcess) }
  default { throw "Unknown Windows automation action: $Action" }
}
`);
async function shell(script) {
  const {stdout}=await execFileAsync(pwsh,['-NoLogo','-NoProfile','-NonInteractive','-Command',
    `$ErrorActionPreference='Stop'; ${script}`],{encoding:'utf8',timeout:30_000,windowsHide:true});
  return stdout.trim();
}
async function native(action,processId,handle='0') {
  const started=Date.now();
  try {
    const {stdout}=await execFileAsync(pwsh,['-NoLogo','-NoProfile','-NonInteractive','-File',
      interopFile,action,String(processId),String(handle)],{encoding:'utf8',timeout:30_000,windowsHide:true});
    return stdout.trim();
  } catch(error) {
    error.message=`Windows automation ${action}: ${error.message}${error.stderr ? `\n${error.stderr.trim()}` : ''}`;
    throw error;
  } finally {
    const elapsed=Date.now()-started;
    if(elapsed>8_000) console.log(`Windows automation ${action} host + interop took ${elapsed} ms; the native message timeout remains 1000 ms.`);
  }
}
// Read the unexpanded raw values, including Task Manager's approval bytes.
// Restore the caller's original values after every success/failure path.
async function startupRegistry() {
  return JSON.parse(await shell(`
    $snapshot = @{}
    foreach ($spec in @(
      @{name='run'; path='Software\\Microsoft\\Windows\\CurrentVersion\\Run'},
      @{name='approval'; path='Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run'}
    )) {
      $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($spec.path)
      try {
        if ($null -eq $key -or -not ($key.GetValueNames() -contains 'Supo Download Manager')) {
          $snapshot[$spec.name] = @{exists=$false}; continue
        }
        $kind = $key.GetValueKind('Supo Download Manager').ToString()
        $value = $key.GetValue('Supo Download Manager',$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        if ($kind -eq 'Binary' -or $kind -eq 'None') { $value = [Convert]::ToBase64String($value) }
        elseif ($kind -eq 'DWord' -or $kind -eq 'QWord') { $value = $value.ToString([Globalization.CultureInfo]::InvariantCulture) }
        $snapshot[$spec.name] = @{exists=$true; kind=$kind; value=$value}
      } finally { if ($null -ne $key) { $key.Dispose() } }
    }
    ConvertTo-Json -InputObject $snapshot -Depth 5 -Compress
  `));
}
async function restoreStartupRegistry(snapshot) {
  const encoded=Buffer.from(JSON.stringify(snapshot),'utf8').toString('base64');
  await shell(`
    $snapshot = ConvertFrom-Json ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('${encoded}')))
    foreach ($spec in @(
      @{name='run'; path='Software\\Microsoft\\Windows\\CurrentVersion\\Run'},
      @{name='approval'; path='Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run'}
    )) {
      $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($spec.path)
      try {
        $saved = $snapshot.($spec.name)
        if (-not $saved.exists) { $key.DeleteValue('Supo Download Manager',$false); continue }
        $value = $saved.value
        switch ($saved.kind) {
          'Binary' { $value = [Convert]::FromBase64String($value) }
          'None' { $value = [Convert]::FromBase64String($value) }
          'DWord' { $value = [int32]::Parse($value,[Globalization.CultureInfo]::InvariantCulture) }
          'QWord' { $value = [int64]::Parse($value,[Globalization.CultureInfo]::InvariantCulture) }
          'MultiString' { $value = [string[]]$value }
        }
        $kind = [Enum]::Parse([Microsoft.Win32.RegistryValueKind],[string]$saved.kind)
        $key.SetValue('Supo Download Manager',$value,$kind)
      } finally { $key.Dispose() }
    }
  `);
}
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
  let originalStartup;
  try {
    originalStartup=await startupRegistry();
    await restoreStartupRegistry({run:{exists:false},approval:{exists:false}});
    fs.writeFileSync(path.join(data,'settings.json'),JSON.stringify(fixtureSettings));
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    const source=`http://127.0.0.1:${server.address().port}`;
    child(process.execPath,[path.join(root,'apps/desktop/node_modules/vite/bin/vite.js'),path.join(root,'apps/desktop'),'--host','127.0.0.1','--port','1420'],'vite');
    await until(async()=>{
      const response=await fetch('http://127.0.0.1:1420/',{signal:AbortSignal.timeout(1_000)});
      return response.ok;
    },'Vite did not start');
    let desktop=child(path.join(root,'target/debug/sdm-desktop.exe'),[],'desktop',{
      ...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9222',
    });
    await until(async()=>{
      const response=await fetch('http://127.0.0.1:9222/json/version',{signal:AbortSignal.timeout(1_000)});
      return response.ok;
    },'WebView2 debugging endpoint did not start',30_000);
    browser=await chromium.connectOverCDP('http://127.0.0.1:9222');
    const pages=()=>browser.contexts().flatMap(context=>context.pages());
    let main=await until(()=>pages().find(page=>!page.url().includes('#')&&page.url().includes(':1420')),'Main WebView was not found');
    await main.getByRole('button',{name:'Settings',exact:true}).waitFor({timeout:20_000});
    // Match this desktop's unowned main window by PID and exact title, then
    // retain that HWND while auxiliary windows are opened and focused.
    let mainWindowHandle=await native('main',desktop.pid);
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

    async function nativeResponsive() {
      // WM_NULL reaches the actual Windows message loop. JavaScript alone
      // can keep responding while a synchronous native IPC handler is stuck.
      assert.equal(await native('responsive',desktop.pid,mainWindowHandle),'responsive');
    }
    async function prompt(pageUrl) {
      await invoke('plugin:event|emit',{event:'sdm:media-prompt',payload:{pageUrl,title:'Home / X'}});
    }

    const setup=main.getByRole('dialog',{name:'Connect your browsers',exact:true});
    await setup.waitFor();
    const installedBrowsers=await invoke('list_browsers');
    assert(installedBrowsers.length>0,'First-run setup should find installed browsers');
    await setup.getByRole('checkbox',{name:`Select ${installedBrowsers[0].name}`,exact:true}).check();
    assert(await setup.getByRole('button',{name:'Set up selected (1)',exact:true}).isEnabled());
    await setup.screenshot({path:path.join(output,'first-run-browser-setup.png')});
    await setup.getByRole('button',{name:'Later',exact:true}).click();
    await setup.waitFor({state:'detached'});
    assert.equal((await invoke('get_settings')).browserSetupCompleted,true);
    assert.equal(JSON.parse(fs.readFileSync(path.join(data,'settings.json'),'utf8')).browserSetupCompleted,true);
    report.push({name:'first-run-browser-detection-selection-and-persisted-dismissal',passed:true,browsers:installedBrowsers.map(b=>b.id)});

    execFileSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',
      '$tokens=$null;$errors=$null;[Management.Automation.Language.Parser]::ParseInput([IO.File]::ReadAllText($env:SDM_BROWSER_INSTALL_SCRIPT),[ref]$tokens,[ref]$errors)|Out-Null;if($errors.Count){$errors|ForEach-Object {$_.Message};exit 1}'],
      {env:{...process.env,SDM_BROWSER_INSTALL_SCRIPT:path.join(root,'apps/desktop/src-tauri/src/browser_install.ps1')}});
    const edge=installedBrowsers.find(b=>b.id==='edge');
    assert(edge,'The Windows image must include Edge for the installation check');
    child(edge.executable,['--no-first-run','--disable-features=msEdgeFirstRunExperience',edge.extensionsPage],'edge-setup-fixture');
    await new Promise(resolve=>setTimeout(resolve,1500));
    const browserSetup=bounded(main.evaluate(()=>window.__TAURI_INTERNALS__.invoke('install_extension',{id:'edge'})),
      'Browser setup exceeded its bounded native timeout',35_000);
    await new Promise(resolve=>setTimeout(resolve,700));
    await invoke('list_downloads');
    await nativeResponsive();
    const setupResult=await browserSetup;
    assert(['automatic','manual','store'].includes(setupResult.method));
    assert.equal(setupResult.browser,'Microsoft Edge');
    assert.equal(JSON.parse(fs.readFileSync(path.join(setupResult.extensionDir,'manifest.json'),'utf8')).version,
      JSON.parse(fs.readFileSync(path.join(root,'extension/dist/chromium/manifest.json'),'utf8')).version);
    let connected=(await invoke('list_browsers')).find(b=>b.id==='edge')?.connected??false;
    if(setupResult.method==='automatic') {
      await until(async()=>{connected=(await invoke('list_browsers')).find(b=>b.id==='edge')?.connected;return connected},
        'An automatically loaded extension did not connect to SDM',15_000);
    }
    report.push({name:'selected-edge-extension-setup-real-browser',passed:true,method:setupResult.method,setupStep:setupResult.setupStep,connected,mainResponsive:true});
    execFileSync(process.execPath,[path.join(root,'.github/scripts/browser-onboarding-ui-smoke.cjs')],{
      env:{...process.env,SDM_UI_BASE:'http://127.0.0.1:1420',SDM_UI_BROWSER:edge.executable,SDM_UI_OUTPUT:path.join(output,'browser-onboarding')},
      stdio:'inherit',timeout:90_000,
    });
    execFileSync(process.execPath,[path.join(root,'.github/scripts/native-connection-ui-smoke.cjs')],{
      env:{...process.env,SDM_UI_BASE:'http://127.0.0.1:1420',SDM_UI_BROWSER:edge.executable,SDM_UI_OUTPUT:path.join(output,'connection-ui')},
      stdio:'inherit',timeout:90_000,
    });

    await main.locator('.idm-toolbar').waitFor();
    const mainDimensions=await main.evaluate(()=>({width:innerWidth,height:innerHeight,
      overflow:document.documentElement.scrollWidth>innerWidth}));
    assert(mainDimensions.width<=882 && mainDimensions.height<=562,
      `Main window is too large: ${JSON.stringify(mainDimensions)}`);
    assert.equal(mainDimensions.overflow,false);
    const workMonitor=await invoke('plugin:window|current_monitor',{label:'main'});
    const outerPosition=await invoke('plugin:window|outer_position',{label:'main'});
    const outerSize=await invoke('plugin:window|outer_size',{label:'main'});
    assert(workMonitor?.workArea,'The native monitor work area is unavailable');
    const work=workMonitor.workArea;
    assert(outerPosition.x>=work.position.x-2 && outerPosition.y>=work.position.y-2
      && outerPosition.x+outerSize.width<=work.position.x+work.size.width+2
      && outerPosition.y+outerSize.height<=work.position.y+work.size.height+2,
      `Main native window exceeds the monitor work area: ${JSON.stringify({outerPosition,outerSize,work})}`);
    await main.screenshot({path:path.join(output,'idm-main.png')});
    report.push({name:'compact-idm-main-window',passed:true,...mainDimensions});

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
      assert(rectangle.width<=482 && rectangle.height<=322,`Confirmation is too large: ${JSON.stringify(rectangle)}`);
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
    await nativeResponsive();
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
    await now.locator('.capture-save input').fill(custom);
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
    assert.equal(await native('restore',desktop.pid,mainWindowHandle),'restored');
    await until(async()=>!await invoke('plugin:window|is_minimized',{label:'main'}),'Main window did not restore');

    // The old extension sent Home rather than the post. Fail promptly with
    // an actionable message, without starting an extractor for the feed.
    await prompt('https://x.com/home');
    await main.getByText('Open the X post containing the video',{exact:false}).waitFor({timeout:8_000});
    await nativeResponsive();
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
    await nativeResponsive();
    await main.screenshot({path:path.join(output,'0-stalled-probe.png')});
    const probePid=Number(fs.readFileSync(pidFile,'utf8'));
    assert(Number.isSafeInteger(probePid)&&probePid>0);
    process.kill(probePid,0); // The fixture must still be alive when Cancel is clicked.
    assert.equal(await main.locator('.media-option').count(),0,'The stalled probe ended before cancellation');
    await main.getByRole('button',{name:'Cancel',exact:true}).click();
    await main.getByRole('dialog').waitFor({state:'hidden',timeout:3_000});
    await until(()=>{
      try { process.kill(probePid,0); return false; }
      catch(error) { if(error.code==='ESRCH') return true; throw error; }
    },'Cancel left the extractor running',8_000);
    report.push({name:'stalled-probe-cancel',passed:true,nativeResponsive:true,childStopped:true});
    await prompt('https://x.com/author/status/987');
    await main.locator('.media-option').first().waitFor({timeout:8_000});
    await nativeResponsive();
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

    // A rejected proxy must not poison either the current downloader or the
    // next launch. Exercise native validation as well as the real Settings UI.
    const settingsFile=path.join(data,'settings.json');
    const settingsBeforeProxy=await invoke('get_settings');
    const diskBeforeProxy=fs.readFileSync(settingsFile);
    const invalidProxy='http://[invalid]';
    await assert.rejects(invoke('save_settings',{settings:{...settingsBeforeProxy,proxy:invalidProxy}}));
    assert.deepEqual(await invoke('get_settings'),settingsBeforeProxy);
    assert.deepEqual(fs.readFileSync(settingsFile),diskBeforeProxy);
    await nativeResponsive();
    report.push({name:'invalid-proxy-native-save-keeps-runtime-and-disk',passed:true,mainResponsive:true});
    await main.getByRole('button',{name:'Settings',exact:true}).click();
    const proxyDialog=main.getByRole('dialog',{name:'Settings',exact:true});
    await proxyDialog.getByRole('tab',{name:'Connection',exact:true}).click();
    await proxyDialog.getByRole('radio',{name:'Custom',exact:true}).check();
    await proxyDialog.getByRole('textbox',{name:'Custom proxy',exact:true}).fill(invalidProxy);
    await proxyDialog.getByRole('button',{name:'Save',exact:true}).click();
    const proxyError=proxyDialog.locator('p.warning[role="alert"]');
    await proxyError.waitFor({timeout:8_000});
    assert((await proxyError.innerText()).trim().length>0);
    // The global toast expires after seven seconds; the field error must stay
    // visible for the user to correct it without having to reproduce the failure.
    await sleep(7_500);
    assert(await proxyError.isVisible());
    assert(await proxyDialog.getByRole('button',{name:'Save',exact:true}).isEnabled());
    assert.deepEqual(await invoke('get_settings'),settingsBeforeProxy);
    assert.deepEqual(fs.readFileSync(settingsFile),diskBeforeProxy);
    await nativeResponsive();
    await main.screenshot({path:path.join(output,'invalid-proxy-settings.png')});
    report.push({name:'invalid-proxy-settings-error-remains-visible',passed:true,persistentInlineError:true});
    await proxyDialog.getByRole('radio',{name:'No proxy',exact:true}).check();
    await proxyDialog.getByRole('button',{name:'Save',exact:true}).click();
    await until(async()=>!(await proxyDialog.count()),'Correcting the proxy did not save Settings');
    assert.equal((await invoke('get_settings')).proxy,'none');
    assert.equal(JSON.parse(fs.readFileSync(settingsFile,'utf8')).proxy,'none');
    report.push({name:'corrected-proxy-settings-save',passed:true,persisted:true});

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
    await until(()=>!fs.existsSync(workspaceA),'Saved completion did not clean its temporary workspace',8_000);
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

    // Restart the actual app, then damage its primary JSON files while it is
    // stopped. The backups below were produced by SDM, not synthetic records.
    const downloadsFile=path.join(data,'downloads.json');
    const restartId=await invoke('add_download',{request:{url:`${source}/restart-paused.bin`,
      fileName:'restart-paused.bin',directory,start:false}});
    await until(async()=>(await invoke('list_downloads')).every(d=>!['downloading','queued'].includes(d.state)),
      'Downloads did not settle before the restart');
    const settingsBeforeRestart=await invoke('get_settings');
    // Also establish a last-good settings backup with the most recent temp root.
    await invoke('save_settings',{settings:settingsBeforeRestart});
    const historyBeforeRestart=await invoke('list_downloads');
    let restartIndex=0;
    async function stopDesktop() {
      const current=desktop;
      const exited=current.exitCode!==null?Promise.resolve():new Promise(resolve=>current.once('exit',resolve));
      // Closing the real main window runs the manager's graceful save/shutdown.
      const closing=invoke('plugin:window|close',{label:'main'}).catch(()=>{});
      await bounded(exited,'The desktop did not exit after closing its main window',15_000);
      await closing;
      await bounded(browser.close(),'Could not disconnect the stopped desktop',3_000).catch(()=>{});
      browser=undefined;
      await until(async()=>{
        try { await fetch('http://127.0.0.1:9222/json/version',{signal:AbortSignal.timeout(500)}); return false; }
        catch { return true; }
      },'The old WebView2 debugging endpoint remained alive',8_000);
    }
    async function dismissRecoveryNotice() {
      return until(async()=>await native('recovery',desktop.pid)==='True',
      'The recovered-data notice was not shown',10_000);
    }
    async function startDesktop(recovered=false,autostart=false) {
      restartIndex+=1;
      desktop=child(path.join(root,'target/debug/sdm-desktop.exe'),autostart?['--autostart']:[],`desktop-restart-${restartIndex}`,{
        ...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9222',
      });
      await until(async()=>{
        const response=await fetch('http://127.0.0.1:9222/json/version',{signal:AbortSignal.timeout(1_000)});
        return response.ok;
      },'Restarted WebView2 debugging endpoint did not start',30_000);
      browser=await chromium.connectOverCDP('http://127.0.0.1:9222');
      main=await until(()=>pages().find(page=>!page.url().includes('#')&&page.url().includes(':1420')),
        'Restarted main WebView was not found');
      await main.getByRole('button',{name:'Settings',exact:true}).waitFor({timeout:20_000});
      if(recovered) await dismissRecoveryNotice();
      mainWindowHandle=await native(autostart?'main-any':'main',desktop.pid);
      assert(/^\d+$/.test(mainWindowHandle) && Number(mainWindowHandle)>0);
      await nativeResponsive();
    }
    const ids=records=>records.map(d=>d.id).sort((a,b)=>a-b);
    await stopDesktop();
    assert(fs.existsSync(`${settingsFile}.bak`),'SDM did not create its settings backup');
    assert(fs.existsSync(`${downloadsFile}.bak`),'SDM did not create its download history backup');
    await startDesktop();
    assert.deepEqual(await invoke('get_settings'),settingsBeforeRestart);
    assert.deepEqual(ids(await invoke('list_downloads')),ids(historyBeforeRestart));
    assert.equal((await invoke('get_download',{id:restartId})).state,'paused');
    assert.equal((await invoke('get_download',{id:fileId})).state,'completed');
    assert.equal(sha(fs.readFileSync(file.path)),sha(bytes));
    assert(!fileRequests.some(r=>r.method==='GET' && !r.probe && r.url==='/restart-paused.bin'));
    report.push({name:'native-restart-retains-settings-and-history',passed:true,contentVerified:true,pausedRetained:true});

    await stopDesktop();
    const settingsBackup=JSON.parse(fs.readFileSync(`${settingsFile}.bak`,'utf8'));
    const historyBackup=JSON.parse(fs.readFileSync(`${downloadsFile}.bak`,'utf8'));
    assert.equal(settingsBackup.tempDir,tempB);
    assert(historyBackup.some(d=>d.id===restartId && d.state==='paused'));
    assert(historyBackup.some(d=>d.id===fileId && d.state==='completed'));
    const damagedSettings=Buffer.from('{"downloadDir":');
    const damagedHistory=Buffer.from('[{"id":');
    fs.writeFileSync(settingsFile,damagedSettings);
    fs.writeFileSync(downloadsFile,damagedHistory);
    await startDesktop(true);
    assert.deepEqual(await invoke('get_settings'),settingsBackup);
    assert.deepEqual(JSON.parse(fs.readFileSync(settingsFile,'utf8')),settingsBackup);
    assert(fs.readdirSync(data).filter(name=>name.startsWith('settings.json.damaged-'))
      .some(name=>fs.readFileSync(path.join(data,name)).equals(damagedSettings)),
      'The damaged settings document was not preserved');
    report.push({name:'damaged-settings-recovers-last-good-backup',passed:true,originalPreserved:true});
    const recoveredHistory=await invoke('list_downloads');
    assert.deepEqual(ids(recoveredHistory),ids(historyBackup));
    assert.equal(recoveredHistory.find(d=>d.id===restartId)?.state,'paused');
    assert.equal(recoveredHistory.find(d=>d.id===fileId)?.state,'completed');
    assert(fs.readdirSync(data).filter(name=>name.startsWith('downloads.json.damaged-'))
      .some(name=>fs.readFileSync(path.join(data,name)).equals(damagedHistory)),
      'The damaged download history was not preserved');
    assert.equal(sha(fs.readFileSync(file.path)),sha(bytes));
    await main.getByRole('button',{name:'Settings',exact:true}).click();
    await main.getByRole('dialog',{name:'Settings',exact:true}).getByRole('button',{name:'Cancel',exact:true}).click();
    await nativeResponsive();
    report.push({name:'damaged-history-recovers-last-good-backup',passed:true,originalPreserved:true,mainResponsive:true});

    // Keep the real updater disconnected from GitHub in this fixture. The
    // signed-package/tamper tests exercise its download and install boundary.
    const updateState=await invoke('get_update_state');
    assert.equal(updateState.supported,true);
    assert.equal(updateState.currentVersion,appVersion);
    assert.equal(updateState.phase,'disabled');
    assert.equal(updateState.downloaded,0);
    assert.equal(updateState.lastChecked,undefined);
    assert.equal((await invoke('get_settings')).autoUpdate,false);
    await main.getByRole('button',{name:'Settings',exact:true}).click();
    const updateDialog=main.getByRole('dialog',{name:'Settings',exact:true});
    await updateDialog.getByRole('tab',{name:'Updates',exact:true}).click();
    const updateCheckbox=updateDialog.getByRole('checkbox',{name:'Automatically download stable updates',exact:true});
    assert(await updateCheckbox.isEnabled());
    assert(!await updateCheckbox.isChecked());
    await updateDialog.getByText(`Installed version: ${appVersion}`,{exact:true}).waitFor();
    await updateDialog.getByText('Automatic updates are off. You can still check manually.',{exact:true}).waitFor();
    // Exercise the preference control while saving only the disabled state;
    // checking the unsaved draft must not start an automatic network request.
    await updateCheckbox.check();
    await updateCheckbox.uncheck();
    await updateDialog.getByRole('button',{name:'Save',exact:true}).click();
    await until(async()=>!(await updateDialog.count()),'The automatic-update preference did not save');
    assert.equal(JSON.parse(fs.readFileSync(settingsFile,'utf8')).autoUpdate,false);
    assert.equal((await invoke('get_update_state')).phase,'disabled');
    report.push({name:'updates-disabled-settings-persist-current-version',passed:true,version:appVersion,externalCheckStarted:false});

    await main.getByRole('button',{name:'Settings',exact:true}).click();
    const startupDialog=main.getByRole('dialog',{name:'Settings',exact:true});
    await startupDialog.getByRole('tab',{name:'General',exact:true}).click();
    const startupCheckbox=startupDialog.getByRole('checkbox',{name:'Start SDM with the computer',exact:true});
    assert(!await startupCheckbox.isChecked());
    await startupCheckbox.check();
    await startupDialog.getByRole('button',{name:'Save',exact:true}).click();
    await until(async()=>!(await startupDialog.count()),'The startup preference did not save');
    assert.equal((await invoke('get_settings')).startWithWindows,true);
    assert.equal(JSON.parse(fs.readFileSync(settingsFile,'utf8')).startWithWindows,true);
    const registered=await startupRegistry();
    assert.equal(registered.run.kind,'String');
    assert.equal(registered.run.value,`"${path.join(root,'target/debug/sdm-desktop.exe')}" --autostart`);
    assert.equal(registered.approval.kind,'Binary');
    assert.deepEqual([...Buffer.from(registered.approval.value,'base64')],[2,0,0,0,0,0,0,0,0,0,0,0]);
    report.push({name:'startup-checkbox-registers-quoted-per-user-command',passed:true,actualOsState:true});

    // Simulate Task Manager disabling an existing registration, including
    // nonzero timestamp bytes. get_settings must reflect the OS immediately.
    const disabledApproval=Buffer.from([3,0,0,0,1,2,3,4,5,6,7,8]);
    await restoreStartupRegistry({...registered,approval:{exists:true,kind:'Binary',value:disabledApproval.toString('base64')}});
    assert.equal((await invoke('get_settings')).startWithWindows,false);
    assert.equal(JSON.parse(fs.readFileSync(settingsFile,'utf8')).startWithWindows,true);
    await main.getByRole('button',{name:'Settings',exact:true}).click();
    const externallyDisabled=main.getByRole('dialog',{name:'Settings',exact:true});
    await externallyDisabled.getByRole('tab',{name:'General',exact:true}).click();
    assert(!await externallyDisabled.getByRole('checkbox',{name:'Start SDM with the computer',exact:true}).isChecked());
    await externallyDisabled.getByRole('button',{name:'Cancel',exact:true}).click();
    report.push({name:'startup-read-reflects-task-manager-disable',passed:true,storedPreferenceDoesNotOverrideOs:true});

    // Deny replacement of settings.json after Windows startup is changed.
    // A failed durable save must restore the exact Run/approval raw values.
    const beforeLockedSave=await invoke('get_settings');
    const registryBeforeLock=await startupRegistry();
    const diskBeforeLock=fs.readFileSync(settingsFile);
    const lockReady=path.join(output,'settings-lock.ready');
    const lockRelease=path.join(output,'settings-lock.release');
    fs.rmSync(lockReady,{force:true}); fs.rmSync(lockRelease,{force:true});
    const lockedPaths=Buffer.from(JSON.stringify({settingsFile,lockReady,lockRelease}),'utf8').toString('base64');
    const lockProcess=child(pwsh,['-NoLogo','-NoProfile','-NonInteractive','-Command',`
      $ErrorActionPreference='Stop'
      $paths = ConvertFrom-Json ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('${lockedPaths}')))
      $lock = [IO.File]::Open($paths.settingsFile,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
      try {
        [IO.File]::WriteAllText($paths.lockReady,'locked')
        $deadline = [DateTime]::UtcNow.AddSeconds(60)
        while (-not [IO.File]::Exists($paths.lockRelease)) {
          if ([DateTime]::UtcNow -gt $deadline) { throw 'Settings lock was not released' }
          Start-Sleep -Milliseconds 100
        }
      } finally { $lock.Dispose() }
    `],'settings-file-lock');
    try {
      await until(()=>fs.existsSync(lockReady),'The settings file lock did not become ready',15_000);
      await assert.rejects(invoke('save_settings',{settings:{...beforeLockedSave,startWithWindows:true}}));
      assert.deepEqual(await startupRegistry(),registryBeforeLock);
      assert.deepEqual(await invoke('get_settings'),beforeLockedSave);
      assert.deepEqual(fs.readFileSync(settingsFile),diskBeforeLock);
      await nativeResponsive();
    } finally {
      fs.writeFileSync(lockRelease,'release');
      await until(()=>lockProcess.exitCode!==null || lockProcess.signalCode!==null,'The settings lock process did not stop',8_000);
    }
    assert.equal(lockProcess.exitCode,0);
    report.push({name:'startup-disk-save-failure-restores-raw-os-values',passed:true,approvalTimestampPreserved:true,runtimeAndDiskUnchanged:true});

    await invoke('save_settings',{settings:{...await invoke('get_settings'),startWithWindows:false}});
    assert.equal((await invoke('get_settings')).startWithWindows,false);
    assert.equal(JSON.parse(fs.readFileSync(settingsFile,'utf8')).startWithWindows,false);
    assert.deepEqual(await startupRegistry(),{run:{exists:false},approval:{exists:false}});
    report.push({name:'startup-disable-removes-run-and-approval-values',passed:true,actualOsState:false});

    const beforeStartupProxy=await invoke('get_settings');
    const registryBeforeProxy=await startupRegistry();
    const diskBeforeStartupProxy=fs.readFileSync(settingsFile);
    await assert.rejects(invoke('save_settings',{settings:{...beforeStartupProxy,proxy:invalidProxy,startWithWindows:true}}));
    assert.deepEqual(await startupRegistry(),registryBeforeProxy);
    assert.deepEqual(await invoke('get_settings'),beforeStartupProxy);
    assert.deepEqual(fs.readFileSync(settingsFile),diskBeforeStartupProxy);
    await nativeResponsive();
    report.push({name:'invalid-proxy-startup-change-keeps-os-runtime-disk',passed:true});

    async function secondaryAutostart(name) {
      const secondary=child(path.join(root,'target/debug/sdm-desktop.exe'),['--autostart'],name,{
        ...process.env,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:'--remote-debugging-port=9222',
      });
      if(secondary.exitCode===null) await bounded(new Promise(resolve=>secondary.once('exit',resolve)),
        'The secondary autostart process did not exit',8_000);
      assert.equal(secondary.exitCode,0);
      await sleep(500);
    }
    await invoke('plugin:window|minimize',{label:'main'});
    await until(()=>invoke('plugin:window|is_minimized',{label:'main'}),'Main SDM window did not minimize before secondary autostart');
    await secondaryAutostart('desktop-autostart-secondary');
    assert.equal(await invoke('plugin:window|is_minimized',{label:'main'}),true);
    await nativeResponsive();
    await stopDesktop();
    await startDesktop(false,true);
    assert.equal(await native('visible',desktop.pid,mainWindowHandle),'False');
    assert.equal(await invoke('plugin:window|is_visible',{label:'main'}),false);
    await nativeResponsive();
    await secondaryAutostart('desktop-autostart-hidden-secondary');
    assert.equal(await native('visible',desktop.pid,mainWindowHandle),'False');
    assert.equal(await invoke('plugin:window|is_visible',{label:'main'}),false);
    report.push({name:'secondary-autostart-keeps-main-minimized-or-hidden',passed:true,existingProcessResponsive:true});

    await invoke('plugin:window|show',{label:'main'});
    assert.equal(await native('restore',desktop.pid,mainWindowHandle),'restored');
    await until(async()=>await invoke('plugin:window|is_visible',{label:'main'})
      && !await invoke('plugin:window|is_minimized',{label:'main'}),'The hidden autostart process did not open normally');
    assert.equal(await native('visible',desktop.pid,mainWindowHandle),'True');
    await nativeResponsive();
    await main.getByRole('button',{name:'Settings',exact:true}).click();
    const restartedUpdates=main.getByRole('dialog',{name:'Settings',exact:true});
    await restartedUpdates.getByRole('tab',{name:'Updates',exact:true}).click();
    assert(!await restartedUpdates.getByRole('checkbox',{name:'Automatically download stable updates',exact:true}).isChecked());
    await restartedUpdates.getByRole('button',{name:'Cancel',exact:true}).click();
    assert.equal((await invoke('get_update_state')).phase,'disabled');
    assert.equal((await invoke('get_update_state')).currentVersion,appVersion);
    report.push({name:'fresh-autostart-hidden-responsive-and-manual-open',passed:true,automaticUpdatesRemainDisabled:true});
    fs.writeFileSync(path.join(output,'results.json'),JSON.stringify({passed:report.length,report},null,2));
    console.log(JSON.stringify({passed:report.length,report},null,2));
  } catch(e) {
    console.error(`::error::Native Windows smoke: ${String(e.message).replaceAll('\n',' ')}`);
    fs.writeFileSync(path.join(output,'failure.txt'),e.stack||String(e));
    for(const name of fs.readdirSync(output).filter(name=>/^(desktop(?:-restart-\d+)?|vite)\.log$/.test(name))) {
      const log=path.join(output,name);
      if(fs.existsSync(log)) console.error(`${name} log:\n${fs.readFileSync(log,'utf8')}`);
    }
    try {
      console.error(await shell("Get-CimInstance Win32_Process | Where-Object { $_.Name -in @('sdm-desktop.exe', 'msedgewebview2.exe') } | Select-Object Name, CommandLine | ConvertTo-Json -Depth 5"));
    } catch {}
    if(browser) for(const [index,page] of browser.contexts().flatMap(c=>c.pages()).entries()) {
      await page.screenshot({path:path.join(output,`failure-${index}.png`),timeout:3_000}).catch(()=>{});
    }
    throw e;
  } finally {
    if(browser) await bounded(browser.close(),'Could not close the smoke browser',3_000).catch(()=>{});
    for(const processHandle of children.reverse()) {
      if(processHandle.pid && processHandle.exitCode===null && processHandle.signalCode===null) {
        try { await execFileAsync('taskkill',['/pid',String(processHandle.pid),'/t','/f'],{timeout:10_000,windowsHide:true}); } catch {}
      }
    }
    try {
      if(originalStartup) await restoreStartupRegistry(originalStartup);
    } finally {
      for(const saved of originalSettings) {
        if(saved.bytes) fs.writeFileSync(saved.file,saved.bytes);
        else fs.rmSync(saved.file,{force:true});
      }
      server.closeAllConnections(); server.close();
    }
  }
})().catch(e=>{console.error(e);process.exit(1)});
