const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const http=require('node:http');
const {chromium}=require(process.env.SDM_SMOKE_PLAYWRIGHT||path.join(process.env.SDM_SMOKE_NODE_MODULES,'playwright-core'));
const base=process.env.SDM_UI_BASE||'http://127.0.0.1:1433';
const output=process.env.SDM_UI_OUTPUT||path.join(process.cwd(),'native-connection-ui');fs.mkdirSync(output,{recursive:true});
const extension=path.resolve('extension/dist/chromium');
const server=http.createServer((req,res)=>{
 const name=new URL(req.url,'http://fixture').pathname.slice(1);
 if(!['popup.html','popup.css','popup.js','icons/48.png'].includes(name)){res.writeHead(404);res.end();return}
 res.setHeader('Content-Type',name.endsWith('.html')?'text/html':name.endsWith('.css')?'text/css':name.endsWith('.js')?'text/javascript':'image/png');
 res.end(fs.readFileSync(path.join(extension,name)));
});
(async()=>{
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 const browser=await chromium.launch({executablePath:process.env.SDM_UI_BROWSER||'/usr/bin/chromium',headless:true,args:['--no-sandbox']});
 const report=[];
 try{
  for(const [lang,width,height]of [['en',880,560],['vi',660,420]]){
   const page=await browser.newPage({viewport:{width,height}});await page.goto(`${base}/?lang=${lang}&theme=light`);
   await page.getByRole('button',{name:lang==='en'?'Settings':'Cài đặt',exact:true}).click();
   const dialog=page.getByRole('dialog');
   await dialog.getByRole('tab',{name:lang==='en'?'Browser & clipboard':'Trình duyệt & clipboard',exact:true}).click();
   const port=dialog.getByText(lang==='en'?'Compatibility port':'Cổng tương thích',{exact:true});assert.equal(await port.isVisible(),false);
   const diagnostics=lang==='en'?'Advanced connection diagnostics':'Chẩn đoán kết nối nâng cao';
   await dialog.getByText(diagnostics,{exact:true}).click();assert.equal(await port.isVisible(),true);
   await dialog.getByText(diagnostics,{exact:true}).click();assert.equal(await port.isVisible(),false);
   await dialog.getByRole('tab',{name:'Torrent',exact:true}).click();
   const torrentPort=dialog.getByText(lang==='en'?'Listening port (0 = automatic)':'Cổng nhận kết nối (0 = tự chọn)',{exact:true});
   assert.equal(await torrentPort.isVisible(),false);
   await dialog.getByRole('tab',{name:lang==='en'?'Browser & clipboard':'Trình duyệt & clipboard',exact:true}).click();
   await page.evaluate(async()=>{const{api}=await import('/src/api.ts');api.retryBrowserConnection=async()=>{throw new Error('Fixture registration denied')}});
   await dialog.getByRole('button',{name:lang==='en'?'Try again':'Thử lại',exact:true}).click();
   await dialog.getByRole('status').filter({hasText:lang==='en'?'needs attention':'cần kiểm tra'}).waitFor();
   assert.equal(await port.isVisible(),false);
   assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
   await page.screenshot({path:path.join(output,`settings-${lang}.png`)});await page.close();
   const messages=JSON.parse(fs.readFileSync(path.join(extension,'_locales',lang,'messages.json'),'utf8'));
   const popup=await browser.newPage({viewport:{width:320,height:640}});
   await popup.addInitScript(messages=>{
    window.connectionFixture={status:null,transport:'none',error:'appNotRunning',canLaunch:true};window.calls=[];
    window.chrome={i18n:{getMessage:key=>messages[key]?.message||key},permissions:{contains:async()=>true,request:async()=>true},
     storage:{local:{get:async defaults=>defaults,set:async()=>{}}},runtime:{sendMessage:async message=>{
      window.calls.push(message.type);
      if(message.type==='open-app'){window.connectionFixture={status:{enabled:true,version:'fixture'},transport:'native',error:null,canLaunch:true};return{ok:true}}
      return window.connectionFixture;
     }}};
   },messages);
   await popup.goto(`http://127.0.0.1:${server.address().port}/popup.html`);
   await popup.locator('#openApp').waitFor();assert.equal(await popup.locator('#port').isVisible(),false);
   await popup.locator('#openApp').click();await popup.locator('#status.ok').waitFor();
   assert((await popup.evaluate(()=>window.calls)).includes('open-app'));
   await popup.evaluate(()=>window.connectionFixture={status:null,transport:'none',error:'connectionError',canLaunch:true});
   await popup.locator('#retryStatus').click();await popup.getByText(messages.statusConnectionError.message,{exact:true}).waitFor();
   await popup.locator('body > details > summary').click();assert.equal(await popup.locator('#port').isVisible(),false);
   await popup.locator('#diagnostics > summary').click();assert.equal(await popup.locator('#port').isVisible(),true);
   await popup.locator('#diagnostics > summary').click();
   assert(await popup.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
   await popup.screenshot({path:path.join(output,`extension-${lang}.png`)});await popup.close();
   report.push({language:lang,desktopPortHidden:true,extensionPortHidden:true,diagnosticsExpandable:true,retryAndLaunch:true});
  }
  fs.writeFileSync(path.join(output,'results.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
 }finally{await browser.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;server.close()});
