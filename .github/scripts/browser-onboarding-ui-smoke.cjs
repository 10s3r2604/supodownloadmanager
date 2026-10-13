const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const os=require('node:os');
const output=process.env.SDM_UI_OUTPUT||path.join(os.tmpdir(),'sdm-browser-onboarding-ui');
fs.mkdirSync(output,{recursive:true});
const base=process.env.SDM_UI_BASE||'http://127.0.0.1:1433';
const {chromium}=require(process.env.SDM_SMOKE_PLAYWRIGHT||path.join(process.env.SDM_SMOKE_NODE_MODULES,'playwright-core'));
(async()=>{
 const browser=await chromium.launch({executablePath:process.env.SDM_UI_BROWSER||'/usr/bin/chromium',headless:true,args:['--no-sandbox']});
 const report=[];
 try{
  for(const [lang,width,height] of [['en',880,560],['vi',660,420]]){
   const page=await browser.newPage({viewport:{width,height}});page.setDefaultTimeout(10000);
   const errors=[];page.on('pageerror',e=>errors.push(e.message));
   await page.goto(`${base}/?lang=${lang}&onboarding=1`);
   const title=lang==='en'?'Connect your browsers':'Kết nối trình duyệt';
   const dialog=page.getByRole('dialog',{name:title,exact:true});await dialog.waitFor();
   const checkbox=name=>dialog.getByRole('checkbox',{name:(lang==='en'?'Select ':'Chọn ')+name,exact:true});
   await checkbox('Google Chrome').waitFor();
   await page.evaluate(async()=>{
    const {api}=await import('/src/api.ts');window.calls=[];window.failEdge=true;window.completeOriginal=api.completeBrowserSetup;
    api.installExtension=async id=>{
     window.calls.push(id);await new Promise(r=>setTimeout(r,60));
     if(id==='edge'&&window.failEdge)throw new Error('Fixture browser install denied');
     return {method:id==='chrome'?'manual':'automatic',browser:id==='chrome'?'Google Chrome':'Microsoft Edge',engine:'chromium',extensionDir:'/fixture/SDM/extension/chromium',extensionsPage:id==='chrome'?'chrome://extensions':'edge://extensions'};
    };
    api.listBrowsers=async()=>[{id:'chrome',name:'Google Chrome',engine:'chromium',executable:'/chrome',extensionsPage:'chrome://extensions',connected:false},{id:'edge',name:'Microsoft Edge',engine:'chromium',executable:'/edge',extensionsPage:'edge://extensions',connected:false},{id:'firefox',name:'Firefox',engine:'firefox',executable:'/firefox',extensionsPage:'about:debugging',connected:false}];
   });
   await checkbox('Google Chrome').check();await checkbox('Microsoft Edge').check();
   await dialog.getByRole('button',{name:lang==='en'?'Set up selected (2)':'Thiết lập đã chọn (2)',exact:true}).click();
   await dialog.getByText('Fixture browser install denied',{exact:true}).waitFor();
   assert.deepEqual(await page.evaluate(()=>window.calls),['chrome','edge']);
   await dialog.getByText('/fixture/SDM/extension/chromium',{exact:true}).waitFor();
   await page.waitForTimeout(3100);
   assert(await checkbox('Google Chrome').isChecked());assert(await checkbox('Microsoft Edge').isChecked());assert(!await checkbox('Firefox').isChecked());
   assert(await dialog.getByText(lang==='en'?'Finish setup in the browser':'Hoàn tất cài đặt trong trình duyệt',{exact:true}).isVisible());
   assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
   assert((await dialog.locator('.dialog-footer').boundingBox()).y+(await dialog.locator('.dialog-footer').boundingBox()).height<=height);
   await page.screenshot({path:path.join(output,`browser-onboarding-${lang}.png`)});
   await page.evaluate(async()=>{window.failEdge=false;const{api}=await import('/src/api.ts');api.completeBrowserSetup=async()=>{throw new Error('Fixture settings write failed')};});
   await dialog.locator('.browser').filter({hasText:'Microsoft Edge'}).getByRole('button').click();
   await dialog.getByText(lang==='en'?'Extension loaded; waiting for connection':'Đã nạp tiện ích; đang chờ kết nối',{exact:true}).waitFor();
   assert.deepEqual(await page.evaluate(()=>window.calls),['chrome','edge','edge']);
   await dialog.locator('.dialog-footer').getByRole('button',{name:lang==='en'?'Close':'Đóng',exact:true}).click();
   await page.getByText('Fixture settings write failed',{exact:true}).waitFor();assert(await dialog.isVisible());
   await page.evaluate(async()=>{const{api}=await import('/src/api.ts');api.completeBrowserSetup=window.completeOriginal});
   await dialog.locator('.dialog-footer').getByRole('button',{name:lang==='en'?'Close':'Đóng',exact:true}).click();
   await dialog.waitFor({state:'detached'});
   assert(await page.evaluate(async()=>{const{api}=await import('/src/api.ts');return (await api.getSettings()).browserSetupCompleted}));
   await page.waitForTimeout(700);assert.equal(await page.getByRole('dialog').count(),0);
   assert.deepEqual(errors,[]);report.push({lang,width,height,firstRun:true,onlySelectedBrowsers:true,pollPreservesSelection:true,partialFailureRetry:true,automaticWaitsForConnection:true,saveFailureKeepsWizard:true,noOverflow:true});
   await page.close();
  }
  const page=await browser.newPage();await page.goto(`${base}/?onboarding=1`);
  const dialog=page.getByRole('dialog',{name:'Connect your browsers',exact:true});await dialog.waitFor();
  await dialog.getByRole('button',{name:'Later',exact:true}).click();await dialog.waitFor({state:'detached'});
  assert(await page.evaluate(async()=>{const{api}=await import('/src/api.ts');return (await api.getSettings()).browserSetupCompleted}));
  report.push({laterSaved:true});await page.close();
  const normal=await browser.newPage();await normal.goto(`${base}/`);await normal.locator('.idm-table').waitFor();assert.equal(await normal.getByRole('dialog').count(),0);report.push({existingInstallationUninterrupted:true});
  fs.writeFileSync(path.join(output,'results.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1)});
