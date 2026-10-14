// Native OLE drop receiver and physical mouse driver for WebView2 file drags.
using System;
using System.Drawing;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

class FileDragFixture {
  const string Title = "SDM native file drop target";
  delegate bool Visit(IntPtr window, IntPtr state);
  [StructLayout(LayoutKind.Sequential)] struct Point { public int X, Y; }
  [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] static extern bool EnumWindows(Visit visit, IntPtr state);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr window, StringBuilder text, int capacity);
  [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr window, ref Point point);
  [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr window);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
  [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
  [DllImport("user32.dll")] static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);

  static IntPtr Window(uint process, string title) {
    IntPtr result = IntPtr.Zero;
    for (int attempt = 0; attempt < 20 && result == IntPtr.Zero; attempt++) {
      EnumWindows((window, state) => {
        uint owner; GetWindowThreadProcessId(window, out owner);
        var text = new StringBuilder(4096); GetWindowText(window, text, text.Capacity);
        if (owner == process && text.ToString() == title) { result = window; return false; }
        return true;
      }, IntPtr.Zero);
      if (result == IntPtr.Zero) Thread.Sleep(100);
    }
    if (result == IntPtr.Zero) throw new Exception("Native window was not found: " + title);
    return result;
  }

  static void Receive(string directory) {
    var form = new Form { Text = Title, StartPosition = FormStartPosition.Manual,
      Location = new System.Drawing.Point(24, 24), Size = new Size(360, 160), TopMost = true,
      AllowDrop = true };
    form.DragEnter += (sender, e) => {
      e.Effect = e.Data.GetDataPresent(DataFormats.FileDrop) && (e.AllowedEffect & DragDropEffects.Copy) != 0
        ? DragDropEffects.Copy : DragDropEffects.None;
      if (e.Effect == DragDropEffects.Copy) File.WriteAllText(Path.Combine(directory, "entered"), "copy");
    };
    form.DragDrop += (sender, e) => {
      var paths = (string[])e.Data.GetData(DataFormats.FileDrop);
      foreach (var file in paths) File.Copy(file, Path.Combine(directory, Path.GetFileName(file)), true);
      File.WriteAllLines(Path.Combine(directory, "received.txt"), paths, new UTF8Encoding(false));
    };
    form.Shown += (sender, e) => File.WriteAllText(Path.Combine(directory, "ready"), "ready");
    Application.Run(form);
  }

  static void Drag(string[] args) {
    var source = Window(uint.Parse(args[1]), args[2]);
    var target = Window(uint.Parse(args[6]), Title);
    if (!SetWindowPos(source, new IntPtr(-1), 420, 50, 0, 0, 0x0041)) throw new Exception("Could not position the source window");
    SetForegroundWindow(source);
    Thread.Sleep(250);
    if (GetForegroundWindow() != source) throw new Exception("The drag source is not in the foreground");
    double scale = double.Parse(args[5], CultureInfo.InvariantCulture);
    var start = new Point { X = (int)Math.Round(double.Parse(args[3], CultureInfo.InvariantCulture) * scale),
      Y = (int)Math.Round(double.Parse(args[4], CultureInfo.InvariantCulture) * scale) };
    var end = new Point { X = 170, Y = 65 };
    if (!ClientToScreen(source, ref start) || !ClientToScreen(target, ref end)) throw new Exception("Could not resolve client coordinates");
    try {
      SetCursorPos(start.X, start.Y);
      mouse_event(2, 0, 0, 0, UIntPtr.Zero);
      Thread.Sleep(150);
      SetCursorPos(start.X + 14, start.Y);
      Thread.Sleep(500);
      for (int step = 1; step <= 20; step++) {
        SetCursorPos(start.X + (end.X - start.X) * step / 20, start.Y + (end.Y - start.Y) * step / 20);
        Thread.Sleep(40);
      }
      // Escape must cancel an actual OLE drag, rather than arrive at an idle
      // WebView if the mouse movement has not yet started native dragging.
      string entered = Path.Combine(args[7], "entered");
      if (args[0] != "missing") {
        for (int attempt = 0; attempt < 50 && !File.Exists(entered); attempt++) Thread.Sleep(100);
        if (!File.Exists(entered)) throw new Exception("Native file drag did not enter the drop target");
      }
      if (args[0] == "cancel") {
        keybd_event(0x1b, 0, 0, UIntPtr.Zero);
        Thread.Sleep(80);
        keybd_event(0x1b, 0, 2, UIntPtr.Zero);
      }
    } finally { mouse_event(4, 0, 0, 0, UIntPtr.Zero); }
    Thread.Sleep(200);
  }

  [STAThread] static int Main(string[] args) {
    try {
      SetProcessDPIAware();
      if (args[0] == "receive") Receive(args[1]);
      else Drag(args);
      return 0;
    } catch (Exception error) { Console.Error.WriteLine(error); return 1; }
  }
}
