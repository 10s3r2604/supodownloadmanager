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
    if(result.stdout!=='loaded') {
      const controls=await run(powershell,['-NoProfile','-NonInteractive','-Mta','-Command',`
        ${providerInitialization}
        $roots=[Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children,[Windows.Automation.Condition]::TrueCondition)
        foreach($window in $roots) {
          try {
            if((Get-Process -Id $window.Current.ProcessId).ProcessName -ne '${browser==='chrome'?'chrome':'msedge'}') {continue}
            $window.Current.Name
            foreach($element in $window.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)) {
              if($element.Current.ControlType -eq [Windows.Automation.ControlType]::Document) {
                $value=$null
                if($element.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern,[ref]$value)) {@{'document'=$element.Current.Name;'url'=$value.Current.Value}|ConvertTo-Json -Compress}
                $text=$null
                if($element.Current.Name -eq 'Extensions' -and $element.TryGetCurrentPattern([Windows.Automation.TextPattern]::Pattern,[ref]$text)) {@{'documentText'=$text.DocumentRange.GetText(2000)}|ConvertTo-Json -Compress}
              }
              if($element.Current.ControlType -eq [Windows.Automation.ControlType]::Button -or $element.Current.Name -match 'Developer|My extensions|Load unpacked|Address|Folder|Select|Cancel' -or $element.Current.ControlType -in @([Windows.Automation.ControlType]::Edit,[Windows.Automation.ControlType]::ComboBox)) {
                @{'name'=$element.Current.Name;'id'=$element.Current.AutomationId;'class'=$element.Current.ClassName;'type'=$element.Current.ControlType.ProgrammaticName;'enabled'=$element.Current.IsEnabled;'patterns'=@($element.GetSupportedPatterns()|ForEach-Object {$_.ProgrammaticName})}|ConvertTo-Json -Compress
              }
            }
          } catch {}
        }`],{timeout:30_000});
      console.log('Remaining native controls:',controls.stdout);
      const policy=await run(powershell,['-NoProfile','-Command',"foreach($key in @('HKLM:\\SOFTWARE\\Policies\\Microsoft\\Edge','HKCU:\\SOFTWARE\\Policies\\Microsoft\\Edge')) {Get-ItemProperty -Path $key -Name ExtensionDeveloperModeSettings,DeveloperToolsAvailability -ErrorAction SilentlyContinue|Select-Object ExtensionDeveloperModeSettings,DeveloperToolsAvailability|ConvertTo-Json -Compress};exit 0"],{timeout:10_000});
      console.log('Explicit Edge developer policies:',policy.stdout.trim());
    }
    if(result.stdout==='loaded') {
      const deadline=Date.now()+15_000;
      while(!connected && Date.now()<deadline) await sleep(200);
      assert(connected,'The extension must really connect after automatic loading');
    }
    console.log(JSON.stringify({method:result.stdout,connected}));
  } finally { await new Promise(resolve=>server.close(resolve)); }
})().catch(error=>{console.error(error);process.exitCode=1});
