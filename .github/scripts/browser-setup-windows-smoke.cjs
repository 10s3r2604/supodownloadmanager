// Test the product's accessibility script against a normal Edge window, without
// remote debugging, profile injection or the --load-extension launch flag.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const {spawn, execFile} = require('node:child_process');
const {promisify} = require('node:util');
const run = promisify(execFile);
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

assert.equal(process.platform, 'win32');
const browser=process.env.SDM_SETUP_TEST_BROWSER || 'chrome';
const executable = [process.env['ProgramFiles(x86)'], process.env.ProgramFiles]
  .filter(Boolean).map(base => path.join(base,browser==='chrome'?'Google/Chrome/Application/chrome.exe':'Microsoft/Edge/Application/msedge.exe'))
  .find(file => fs.existsSync(file));
assert(executable, `The test image must include ${browser}`);
const directory = path.join(process.env.RUNNER_TEMP, 'SDM tiện ích', 'chromium');
function copyBundle(source,target) {
  fs.mkdirSync(target,{recursive:true});
  for(const entry of fs.readdirSync(source,{withFileTypes:true})) {
    const from=path.join(source,entry.name), to=path.join(target,entry.name);
    if(entry.isDirectory()) copyBundle(from,to); else fs.copyFileSync(from,to);
  }
}
copyBundle(path.resolve('extension/dist/chromium'), directory);
const request = {executable, directory, page:`${browser}://extensions`, engine:'chromium',
  version:JSON.parse(fs.readFileSync(path.join(directory,'manifest.json'),'utf8')).version};
let connected = false;
let connectedVersion = null;
const server = http.createServer((req,res) => {
  if (req.url === '/api/ping' && req.headers['x-sdm-client'] === 'extension'
    && req.headers['x-sdm-browser'] === browser) { connected = true; connectedVersion = req.headers['x-sdm-extension-version']; }
  res.setHeader('Content-Type','application/json');
  res.end(JSON.stringify({enabled:true, version:'browser-setup-fixture'}));
});
const powershell = path.join(process.env.SystemRoot,'System32/WindowsPowerShell/v1.0/powershell.exe');
const productScript=fs.readFileSync('apps/desktop/src-tauri/src/browser_install.ps1','utf8');
const providerInitialization=productScript.slice(productScript.indexOf('Add-Type -AssemblyName UIAutomationClient'), productScript.indexOf("$script:stage = 'locate-window'"));
async function automation(script) {
  return new Promise((resolve,reject) => {
    const child = spawn(powershell,['-NoProfile','-NonInteractive','-Mta','-Command',script],{windowsHide:true});
    let stdout='',stderr='';
    child.stdout.on('data',chunk => {stdout+=chunk});
    child.stderr.on('data',chunk => {stderr+=chunk});
    let timedOut=false;
    const timeout=setTimeout(()=>{timedOut=true;child.kill();resolve({code:0,stdout:'manual:timeout',stderr,timeout:true})},30_000);
    child.on('error',error=>{clearTimeout(timeout);reject(error)});
    child.on('close',code=>{clearTimeout(timeout);if(!timedOut) resolve({code,stdout,stderr})});
    child.stdin.end(JSON.stringify(request));
  });
}
(async()=>{
  await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(17360,'127.0.0.1',resolve)});
  try {
    // Suppress only the test image's first-use welcome page. The product opens
    // the management URL with no extra browser launch flags.
    spawn(executable,['--no-first-run','--no-default-browser-check','--start-maximized','--disable-features=msEdgeFirstRunExperience',request.page],{stdio:'ignore'}).unref();
    await sleep(2000);
    await run(powershell,['-NoProfile','-NonInteractive','-Mta','-Command',`
      ${providerInitialization}
      $clock=[Diagnostics.Stopwatch]::StartNew()
      while($clock.Elapsed.TotalSeconds -lt 10) {
        foreach($element in [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.Condition]::TrueCondition)) {
          try {if((Get-Process -Id $element.Current.ProcessId).ProcessName -eq '${browser==='chrome'?'chrome':'msedge'}') {exit 0}} catch {}
        }
        Start-Sleep -Milliseconds 250
      }
      exit 1`],{timeout:30_000});
    // Match the app's open_in call after the test browser is initialized.
    spawn(executable,[request.page],{stdio:'ignore'}).unref();
    await sleep(500);
    const source=productScript;
    // The only instrumentation reports exceptions from this isolated fixture;
    // browser interaction and the success/fallback result are the product code.
    const instrumented=source
      .replaceAll('} catch { }','} catch { [Console]::Error.WriteLine($_.Exception.GetBaseException().ToString()) }')
      .replace(/(\$(?:script:)?stage = '[^']+')/g, '$1; [Console]::Error.WriteLine($script:stage)');
    const result=await automation(instrumented);
    console.log(JSON.stringify(result));
    assert.equal(result.code,0);
    if(browser==='chrome') assert.equal(result.stdout,'loaded','Normal Chrome controls must load the extension automatically');
    if(result.stdout==='loaded') {
      const deadline=Date.now()+15_000;
      while(!connected && Date.now()<deadline) await sleep(200);
      assert(connected,'The extension must really connect after automatic loading');
    }
    console.log(JSON.stringify({method:result.stdout,connected}));
    if (browser === 'chrome') {
      // Refresh the same unpacked identity through the product's Reload flow.
      const manifestFile=path.join(directory,'manifest.json');
      const manifest=JSON.parse(fs.readFileSync(manifestFile,'utf8'));
      const parts=manifest.version.split('.').map(Number);parts[parts.length-1]+=1;
      manifest.version=parts.join('.');fs.writeFileSync(manifestFile,JSON.stringify(manifest));
      request.version=manifest.version;request.updating=true;connected=false;
      const reloaded=await automation(instrumented);console.log(JSON.stringify(reloaded));
      assert.equal(reloaded.stdout,'loaded','Updating must reload the existing extension through normal Chrome controls');
      const deadline=Date.now()+15000;
      while ((!connected || connectedVersion!==manifest.version) && Date.now()<deadline) await sleep(200);
      assert.equal(connectedVersion,manifest.version,'The reloaded worker must report the new manifest version');
      console.log(JSON.stringify({existingExtensionUpdated:true,version:connectedVersion}));
    }
  } finally { await new Promise(resolve=>server.close(resolve)); }
})().catch(error=>{console.error(error);process.exitCode=1});
