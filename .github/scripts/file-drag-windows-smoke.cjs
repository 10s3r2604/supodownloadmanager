// Verify the real completion icons and native OLE FileDrop data across layouts.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {execFile, execFileSync, spawn} = require('node:child_process');
const {promisify} = require('node:util');
const execFileAsync = promisify(execFile);

module.exports = async function verifyFileDrag({invoke, pages, processId, file, output, until, report}) {
  const directory = path.join(output, 'file-drop');
  fs.mkdirSync(directory, {recursive:true});
  const exe = path.join(directory, 'FileDragFixture.exe');
  execFileSync(path.join(process.env.WINDIR, 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'), [
    '/nologo', '/target:exe', '/r:System.Windows.Forms.dll', '/r:System.Drawing.dll', `/out:${exe}`,
    path.join(process.cwd(), '.github/scripts/file-drag-windows-fixture.cs'),
  ]);
  const log = fs.openSync(path.join(directory, 'receiver.log'), 'w');
  const receiver = spawn(exe, ['receive', directory], {stdio:['ignore',log,log]});
  fs.closeSync(log);
  const settings = await invoke('get_settings');
  const original = fs.readFileSync(file.path);
  const received = path.join(directory, 'received.txt');
  let current;
  try {
    await until(() => fs.existsSync(path.join(directory, 'ready')), 'Native OLE receiver did not start');
    for (const style of ['graph', 'ring', 'classic']) {
      await invoke('save_settings', {settings:{...settings,progressStyle:style}});
      await invoke('open_progress_window', {id:file.id});
      current = await until(() => pages().find(page => page.url().endsWith(`#progress/${file.id}`)), 'Completion window was not created');
      const icon = current.getByRole('button', {name:'Drag file out',exact:true});
      await icon.waitFor({timeout:10_000});
      await current.evaluate(async () => {
        // Tauri's IPC internals are read-only. Observe the app API while still
        // calling its real native implementation and awaiting its actual reply.
        const {api} = await import('/src/api.ts');
        const original = api.dragDownload.bind(api);
        window.fileDragSmoke = {calls:0,settled:0,errors:0};
        api.dragDownload = async (...args) => {
          window.fileDragSmoke.calls++;
          try { return await original(...args); }
          catch (error) { window.fileDragSmoke.errors++; throw error; }
          finally { window.fileDragSmoke.settled++; }
        };
      });
      // Ordinary clicks must neither start a drag nor move the native window.
      await icon.click();
      assert.equal(await current.evaluate(() => window.fileDragSmoke.calls), 0);
      await current.screenshot({path:path.join(output, `file-drag-${style}.png`)});
      async function gesture(action) {
        const box = await icon.boundingBox();
        assert(box);
        const scale = await current.evaluate(() => window.devicePixelRatio);
        await execFileAsync(exe, [action,String(processId),await current.title(),
          String(box.x + box.width / 2),String(box.y + box.height / 2),String(scale),String(receiver.pid)],
        {timeout:15_000,encoding:'utf8'});
      }
      fs.rmSync(received, {force:true});
      await gesture('drag');
      await until(() => fs.existsSync(received), `${style}: Native target did not receive FileDrop data`);
      const paths = fs.readFileSync(received, 'utf8').trim().split(/\r?\n/);
      assert.deepEqual(paths, [file.path]);
      assert.deepEqual(fs.readFileSync(path.join(directory, path.basename(file.path))), original);
      assert.deepEqual(fs.readFileSync(file.path), original, 'Dragging must preserve the source download');
      await until(() => current.evaluate(() => window.fileDragSmoke.settled === 1), 'Native drag IPC did not finish');
      assert.deepEqual(await current.evaluate(() => window.fileDragSmoke), {calls:1,settled:1,errors:0});
      assert.equal((await invoke('get_download', {id:file.id})).state, 'completed');
      if (style === 'classic') {
        fs.rmSync(received, {force:true});
        await gesture('cancel');
        await until(() => current.evaluate(() => window.fileDragSmoke.settled === 2), 'Cancelled native drag IPC did not finish');
        assert.equal(fs.existsSync(received), false, 'Cancelling a drag must not deliver a file');
        assert.deepEqual(fs.readFileSync(file.path), original);
        assert.equal(await current.evaluate(() => window.fileDragSmoke.errors), 0);
        // Missing source files produce visible, recoverable feedback.
        const moved = `${file.path}.smoke-moved`;
        fs.renameSync(file.path, moved);
        try {
          await gesture('drag');
          await current.getByRole('alert').filter({hasText:'Could not drag the file'}).waitFor();
          assert.equal(fs.existsSync(received), false);
        } finally { fs.renameSync(moved, file.path); }
      }
      report.push({name:`native-file-drag-${style}`,copyVerified:true,sourcePreserved:true});
      await Promise.all([current.waitForEvent('close'),current.getByRole('button',{name:'Close',exact:true}).click()]);
      current = null;
    }
    await assert.rejects(invoke('drag_download', {id:Number.MAX_SAFE_INTEGER}), /unknown download/);
  } finally {
    if (current && !current.isClosed()) await current.getByRole('button',{name:'Close',exact:true}).click().catch(() => {});
    receiver.kill();
    await invoke('save_settings', {settings});
  }
};
