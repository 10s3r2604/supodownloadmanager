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
const browser=process.env.SDM_SETUP_TEST_BROWSER || 'edge';
const executable = [process.env['ProgramFiles(x86)'], process.env.ProgramFiles]
  .filter(Boolean).map(base => path.join(base,browser==='chrome'?'Google/Chrome/Application/chrome.exe':'Microsoft/Edge/Application/msedge.exe'))
  .find(file => fs.existsSync(file));
assert(executable, 'The test image must include Edge');
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
const server = http.createServer((req,res) => {
  if (req.url === '/api/ping' && req.headers['x-sdm-client'] === 'extension'
    && req.headers['x-sdm-browser'] === browser) connected = true;
  res.setHeader('Content-Type','application/json');
  res.end(JSON.stringify({enabled:true, version:'browser-setup-fixture'}));
});
const powershell = path.join(process.env.SystemRoot,'System32/WindowsPowerShell/v1.0/powershell.exe');
async function automation(script) {
  return new Promise((resolve,reject) => {
    const child = spawn(powershell,['-NoProfile','-NonInteractive','-Mta','-Command',script],{windowsHide:true});
    let stdout='',stderr='';
    child.stdout.on('data',chunk => {stdout+=chunk});
    child.stderr.on('data',chunk => {stderr+=chunk});
    const timeout=setTimeout(()=>{console.log(JSON.stringify({stdout,stderr,timeout:true}));child.kill();reject(new Error('Accessibility helper exceeded 24 seconds'))},24_000);
    child.on('error',error=>{clearTimeout(timeout);reject(error)});
    child.on('close',code=>{clearTimeout(timeout);resolve({code,stdout,stderr})});
    child.stdin.end(JSON.stringify(request));
  });
}
(async()=>{
  await new Promise((resolve,reject)=>{server.once('error',reject);server.listen(17360,'127.0.0.1',resolve)});
  try {
    // Suppress only the test image's first-use welcome page. The product opens
    // the management URL with no extra browser launch flags.
    spawn(executable,['--no-first-run','--no-default-browser-check','--disable-features=msEdgeFirstRunExperience',request.page],{stdio:'ignore'}).unref();
    await sleep(2000);
    const diagnosis = await run(powershell,['-NoProfile','-NonInteractive','-Mta','-Command',`
      Add-Type -AssemblyName UIAutomationClient
      Add-Type -AssemblyName UIAutomationTypes
      $children=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.Condition]::TrueCondition)
      foreach($element in $children) {
        try {
          if((Get-Process -Id $element.Current.ProcessId).ProcessName -eq '${browser==='chrome'?'chrome':'msedge'}') {
            $element.Current.Name
            $element.Current.ControlType.ProgrammaticName
            $element.Current.ClassName
            $element.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Document))|ForEach-Object {$_.Current.Name}
            foreach($edit in $element.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::IsValuePatternAvailableProperty,$true))) {
              $provider=$null
              if($edit.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern,[ref]$provider)) {
                @{'type'=$edit.Current.ControlType.ProgrammaticName;'name'=$edit.Current.Name;'value'=$provider.Current.Value}|ConvertTo-Json -Compress
              }
              $provider=$null
              if($edit.TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$provider)) {
                @{'name'=$edit.Current.Name;'text'=$provider.DocumentRange.GetText(-1)}|ConvertTo-Json -Compress
              }
            }
          }
        } catch {}
      }`],{timeout:30_000});
    console.log('Browser accessibility control types, classes and value counts:',diagnosis.stdout.trim());
    // Match the app's open_in call after the test browser is initialized.
    spawn(executable,[request.page],{stdio:'ignore'}).unref();
    await sleep(500);
    const source=fs.readFileSync('apps/desktop/src-tauri/src/browser_install.ps1','utf8');
    // The only instrumentation reports exceptions from this isolated fixture;
    // browser interaction and the success/fallback result are the product code.
    const instrumented=source
      .replaceAll('} catch { }','} catch { [Console]::Error.WriteLine($_.Exception.GetType().Name + ": " + $_.Exception.Message) }')
      .replace(/(\$(?:script:)?stage = '[^']+')/g, '$1; [Console]::Error.WriteLine($script:stage)');
    const result=await automation(instrumented);
    console.log(JSON.stringify(result));
    assert.equal(result.code,0);
    if(result.stdout==='loaded') {
      const deadline=Date.now()+15_000;
      while(!connected && Date.now()<deadline) await sleep(200);
      assert(connected,'The extension must really connect after automatic loading');
    }
    console.log(JSON.stringify({method:result.stdout,connected}));
  } finally { await new Promise(resolve=>server.close(resolve)); }
})().catch(error=>{console.error(error);process.exitCode=1});
