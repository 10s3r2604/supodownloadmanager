import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
const code=readFileSync(new URL('../src/background.js',import.meta.url),'utf8');
const id='abcdefghijklmnopabcdefghijklmnop';
async function worker(native,{legacyReady=true}={}) {
 const calls=[], legacy=[];let listener;
 const event={addListener(){}};
 const chrome={
  runtime:{id,getURL:path=>`chrome-extension://${id}/${path}`,sendNativeMessage:async(host,message)=>{calls.push({host,message});return native(message)},onInstalled:event,onStartup:event,onMessage:{addListener(fn){listener=fn}}},
  storage:{local:{async get(defaults){return {...defaults,port:19999}}}},
  cookies:{async getAll(){return [{name:'session',value:'fixture'}]}},
  downloads:{onCreated:event,onChanged:event,onDeterminingFilename:event},
  contextMenus:{onClicked:event},tabs:{onActivated:event,onUpdated:event},alarms:{create(){},onAlarm:event},
  action:{setTitle(){}},i18n:{getMessage:key=>key},
 };
 vm.runInNewContext(code,{chrome,navigator:{userAgent:'Google Chrome',userAgentData:{brands:[{brand:'Google Chrome'}]}},URL,AbortSignal,TypeError,setTimeout,clearTimeout,
  async fetch(url,options){legacy.push({url,options});if(!legacyReady)throw new TypeError('App offline');return Response.json({enabled:true,version:'0.7.3'})}
 });
 const send=(value,sender={})=>new Promise(resolve=>{if(!listener(value,sender,resolve))resolve('rejected')});
 await send({type:'status'});return {calls,legacy,send};
}
const ready=()=>({status:200,body:JSON.stringify({enabled:true,media:true,mediaProbeJobs:true,version:'0.7.4'})});
test('native transport ignores an old custom port and preserves file metadata',async()=>{
 const app=await worker(message=>message.path==='/api/ping'?ready():{status:202,body:''});
 const result=await app.send({type:'magnet',url:'magnet:?xt=urn:btih:fixture'},{tab:{url:'https://fixture.example'}});
 assert.equal(result.ok,true);assert.equal(app.legacy.length,0);
 const request=app.calls.find(call=>call.message.path==='/api/download');
 assert.equal(request.host,'com.supodownloadmanager.bridge');assert.equal(request.message.browser,'chrome');
 assert.equal(request.message.body.url,'magnet:?xt=urn:btih:fixture');assert.equal(request.message.body.referrer,'https://fixture.example');
 const status=await app.send({type:'connection-status'});assert.equal(status.transport,'native');
});
test('an unregistered host retains older desktops automatically',async()=>{
 const app=await worker(()=>{throw new Error('Specified native messaging host not found.');});
 const status=await app.send({type:'connection-status'});
 assert.equal(status.transport,'compatibility');assert.equal(status.status.version,'0.7.3');
 assert(app.legacy.every(call=>call.url.startsWith('http://127.0.0.1:19999/')));
 assert.equal(app.legacy[0].options.headers['X-SDM-Extension-ID'],id);
});
test('a lost native reply cannot submit the same download through HTTP',async()=>{
 const app=await worker(message=>{if(message.path==='/api/ping')return ready();throw new Error('Native host has exited.');});
 const answer=await app.send({type:'magnet',url:'magnet:?xt=urn:btih:fixture'},{tab:{url:'https://fixture.example'}});
 assert.equal(answer.ok,false);assert.equal(app.calls.filter(call=>call.message.path==='/api/download').length,1);assert.equal(app.legacy.length,0);
});
test('a closed app is distinct from a host configuration error; launch is restricted to extension pages',async()=>{
 const app=await worker(message=>message.action==='launch'?{launched:true}:{error:'appNotRunning'},{legacyReady:false});
 const status=await app.send({type:'connection-status'});assert.equal(status.error,'appNotRunning');assert.equal(status.canLaunch,true);
 assert.equal(await app.send({type:'open-app'},{tab:{url:'https://example.com'}}),'rejected');
 assert.equal((await app.send({type:'open-app'})).ok,true);assert(app.legacy.every(call=>!call.options.body));
 assert.equal((await app.send({type:'open-app'},{url:`chrome-extension://${id}/popup.html`,tab:{id:7}})).ok,true);
});
test('an old executable in a stale registration is detected with GET before handing over once through compatibility',async()=>{
 const app=await worker(()=>{throw new Error('Native host has exited.');});
 assert.equal((await app.send({type:'connection-status'})).transport,'compatibility');
 assert.equal((await app.send({type:'magnet',url:'magnet:?xt=urn:btih:fixture'},{tab:{url:'https://fixture.example'}})).ok,true);
 assert(app.calls.every(call=>call.message.path==='/api/ping'));
 assert.equal(app.legacy.filter(call=>call.options.body).length,1);
});
