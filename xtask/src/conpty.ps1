# Runs a console program inside a Windows pseudo-console (ConPTY) and lets
# xtask drive its keyboard: ArgyllCMS's spotread reads keys from the console
# itself (not from a pipe), so one long-running spotread can take a reading
# per key press (about 2 s) instead of one start-up per reading (about 10 s).
#
# xtask writes the program's command line in RP_PTY_CMD and its folder in
# RP_PTY_CWD, then talks over this script's stdin/stdout, one line each:
#   KEY <text>        send <text> as key presses (no reply)
#   WAIT <ms> <text>  wait until the program's output holds <text>; reply
#                     "OUT <output so far, one line, escapes removed>" or
#                     "TIMEOUT <output so far>"; the output is then cleared
#   QUIT              close the console and end
# Everything the program prints stays inside the pseudo-console.

$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32.SafeHandles;

public static class RpPty {
    [StructLayout(LayoutKind.Sequential)]
    public struct COORD { public short X; public short Y; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct STARTUPINFO {
        public int cb; public string lpReserved; public string lpDesktop; public string lpTitle;
        public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
        public short wShowWindow, cbReserved2;
        public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct STARTUPINFOEX { public STARTUPINFO StartupInfo; public IntPtr lpAttributeList; }

    [StructLayout(LayoutKind.Sequential)]
    public struct PROCESS_INFORMATION { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern int CreatePseudoConsole(COORD size, SafeFileHandle input, SafeFileHandle output, uint flags, out IntPtr pc);
    [DllImport("kernel32.dll")]
    static extern void ClosePseudoConsole(IntPtr pc);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool CreatePipe(out SafeFileHandle read, out SafeFileHandle write, IntPtr attrs, int size);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool InitializeProcThreadAttributeList(IntPtr list, int count, int flags, ref IntPtr size);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool UpdateProcThreadAttribute(IntPtr list, uint flags, IntPtr attr, IntPtr value, IntPtr size, IntPtr prev, IntPtr ret);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern bool CreateProcessW(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit,
        uint flags, IntPtr env, string cwd, ref STARTUPINFOEX si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateProcess(IntPtr process, uint code);

    const uint EXTENDED_STARTUPINFO_PRESENT = 0x00080000;
    const int STARTF_USESTDHANDLES = 0x100;
    static readonly IntPtr PSEUDOCONSOLE = (IntPtr)0x00020016;

    static IntPtr pc;
    static IntPtr process;
    static System.IO.FileStream keys;
    static readonly StringBuilder output = new StringBuilder();

    public static void Start(string cmd, string cwd) {
        SafeFileHandle inRead, inWrite, outRead, outWrite;
        if (!CreatePipe(out inRead, out inWrite, IntPtr.Zero, 0)) throw new Exception("CreatePipe in");
        if (!CreatePipe(out outRead, out outWrite, IntPtr.Zero, 0)) throw new Exception("CreatePipe out");
        COORD size; size.X = 120; size.Y = 40;
        int hr = CreatePseudoConsole(size, inRead, outWrite, 0, out pc);
        if (hr != 0) throw new Exception("CreatePseudoConsole " + hr);
        inRead.Dispose();
        outWrite.Dispose();

        IntPtr bytes = IntPtr.Zero;
        InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref bytes);
        IntPtr list = Marshal.AllocHGlobal(bytes);
        if (!InitializeProcThreadAttributeList(list, 1, 0, ref bytes)) throw new Exception("attribute list");
        if (!UpdateProcThreadAttribute(list, 0, PSEUDOCONSOLE, pc, (IntPtr)IntPtr.Size, IntPtr.Zero, IntPtr.Zero))
            throw new Exception("attribute");
        STARTUPINFOEX si = new STARTUPINFOEX();
        si.StartupInfo.cb = Marshal.SizeOf(typeof(STARTUPINFOEX));
        // Our own stdin/stdout are pipes to xtask: without this flag (and no
        // handles) the child would take them instead of the pseudo-console.
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.lpAttributeList = list;
        PROCESS_INFORMATION pi;
        if (!CreateProcessW(null, new StringBuilder(cmd), IntPtr.Zero, IntPtr.Zero, false,
                EXTENDED_STARTUPINFO_PRESENT, IntPtr.Zero, cwd, ref si, out pi))
            throw new Exception("CreateProcess " + Marshal.GetLastWin32Error());
        process = pi.hProcess;
        keys = new System.IO.FileStream(inWrite, System.IO.FileAccess.Write);
        var reader = new System.IO.FileStream(outRead, System.IO.FileAccess.Read);
        var t = new Thread(() => {
            var buf = new byte[4096];
            try {
                int n;
                while ((n = reader.Read(buf, 0, buf.Length)) > 0) {
                    lock (output) { output.Append(Encoding.UTF8.GetString(buf, 0, n)); }
                }
            } catch (Exception) { }
        });
        t.IsBackground = true;
        t.Start();
    }

    public static void Key(string text) {
        var b = Encoding.UTF8.GetBytes(text);
        keys.Write(b, 0, b.Length);
        keys.Flush();
    }

    /// The output so far, if it holds `text` (or `force`), then cleared.
    public static string Take(string text, bool force) {
        lock (output) {
            string s = output.ToString();
            if (!force && s.IndexOf(text, StringComparison.Ordinal) < 0) return null;
            output.Clear();
            return s;
        }
    }

    public static void Stop() {
        try { Key("q"); Thread.Sleep(300); } catch (Exception) { }
        if (process != IntPtr.Zero) TerminateProcess(process, 0);
        if (pc != IntPtr.Zero) ClosePseudoConsole(pc);
    }
}
'@

function Clean([string]$s) {
    $s = [regex]::Replace($s, '\x1b\][^\x07]*\x07', '')
    $s = [regex]::Replace($s, '\x1b\[[0-9;?]*[ -/]*[@-~]', '')
    $s = [regex]::Replace($s, '[\x00-\x1f]+', ' ')
    return $s.Trim()
}

[RpPty]::Start($env:RP_PTY_CMD, $env:RP_PTY_CWD)
[Console]::Out.WriteLine('STARTED')
[Console]::Out.Flush()
try {
    while ($null -ne ($line = [Console]::In.ReadLine())) {
        if ($line.StartsWith('KEY ')) {
            [RpPty]::Key($line.Substring(4))
        } elseif ($line.StartsWith('WAIT ')) {
            $parts = $line.Split(' ', 3)
            $until = [DateTime]::UtcNow.AddMilliseconds([int]$parts[1])
            $got = $null
            while ($null -eq $got -and [DateTime]::UtcNow -lt $until) {
                $got = [RpPty]::Take($parts[2], $false)
                if ($null -eq $got) { Start-Sleep -Milliseconds 20 }
            }
            if ($null -eq $got) {
                [Console]::Out.WriteLine('TIMEOUT ' + (Clean ([RpPty]::Take('', $true))))
            } else {
                [Console]::Out.WriteLine('OUT ' + (Clean $got))
            }
            [Console]::Out.Flush()
        } elseif ($line -eq 'QUIT') {
            break
        }
    }
} finally {
    [RpPty]::Stop()
}
