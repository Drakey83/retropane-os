// The R61 cache-free probe (2026-10-26): Add-Type -TypeDefinition (Get-Content -Raw ShellIcon.cs); [ShellIcon]::Run("<shell folder parsing name>")
using System;
using System.Runtime.InteropServices;
using System.Text;

// Asks the shell, without its icon cache, which icon each child of a shell
// folder uses: IExtractIconW::GetIconLocation (file, index, flags).
[ComImport, Guid("000214E6-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IShellFolder {
    void ParseDisplayName(IntPtr hwnd, IntPtr bc, [MarshalAs(UnmanagedType.LPWStr)] string name, out uint eaten, out IntPtr pidl, ref uint attrs);
    void EnumObjects(IntPtr hwnd, uint flags, out IntPtr e);
    void BindToObject(IntPtr pidl, IntPtr bc, ref Guid riid, out IntPtr ppv);
    void BindToStorage(IntPtr pidl, IntPtr bc, ref Guid riid, out IntPtr ppv);
    [PreserveSig] int CompareIDs(IntPtr l, IntPtr a, IntPtr b);
    void CreateViewObject(IntPtr hwnd, ref Guid riid, out IntPtr ppv);
    void GetAttributesOf(uint n, [MarshalAs(UnmanagedType.LPArray)] IntPtr[] pidls, ref uint attrs);
    [PreserveSig] int GetUIObjectOf(IntPtr hwnd, uint n, [MarshalAs(UnmanagedType.LPArray)] IntPtr[] pidls, ref Guid riid, IntPtr reserved, out IntPtr ppv);
    void GetDisplayNameOf(IntPtr pidl, uint flags, IntPtr name);
}

[ComImport, Guid("000214FA-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IExtractIconW {
    [PreserveSig] int GetIconLocation(uint flags, StringBuilder file, int cch, out int index, out uint outFlags);
    [PreserveSig] int Extract([MarshalAs(UnmanagedType.LPWStr)] string file, uint index, out IntPtr large, out IntPtr small, uint size);
}

[ComImport, Guid("000214F2-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IEnumIDList {
    [PreserveSig] int Next(uint n, out IntPtr pidl, out uint fetched);
}

public static class ShellIcon {
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    static extern int SHParseDisplayName(string name, IntPtr bc, out IntPtr pidl, uint sfgao, out uint attrs);
    [DllImport("shell32.dll")]
    static extern int SHBindToObject(IntPtr psf, IntPtr pidl, IntPtr bc, ref Guid riid, out IntPtr ppv);
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    static extern int SHGetNameFromIDList(IntPtr pidl, uint sigdn, out IntPtr name);
    [DllImport("shell32.dll")]
    static extern IntPtr ILCombine(IntPtr a, IntPtr b);

    [StructLayout(LayoutKind.Sequential)] struct ICONINFO { public int fIcon; public int x; public int y; public IntPtr mask; public IntPtr color; }
    [DllImport("user32.dll")] static extern bool GetIconInfo(IntPtr h, out ICONINFO ii);
    [DllImport("gdi32.dll")] static extern int GetObject(IntPtr h, int n, byte[] buf);
    [DllImport("gdi32.dll")] static extern int GetBitmapBits(IntPtr h, int n, byte[] buf);

    // The icon's colour pixels: size and a 64-bit FNV-1a fingerprint, and
    // the raw pixels saved for a look (temp folder).
    static string Pixels(IntPtr icon, string name) {
        ICONINFO ii; if (!GetIconInfo(icon, out ii)) return "no info";
        var bm = new byte[32]; GetObject(ii.color, 32, bm);
        int w = BitConverter.ToInt32(bm, 4), h = BitConverter.ToInt32(bm, 8), stride = BitConverter.ToInt32(bm, 12);
        var px = new byte[stride * h]; GetBitmapBits(ii.color, px.Length, px);
        ulong f = 14695981039346656037UL; foreach (var b in px) { f ^= b; f *= 1099511628211UL; }
        System.IO.File.WriteAllBytes(System.IO.Path.Combine(System.IO.Path.GetTempPath(), "rp-icon-" + name + ".bgra"), px);
        return string.Format("{0}x{1} fnv {2:x16}", w, h, f);
    }

    static string Name(IntPtr abs) {
        IntPtr p; if (SHGetNameFromIDList(abs, 0, out p) != 0) return "?";
        string s = Marshal.PtrToStringUni(p); Marshal.FreeCoTaskMem(p); return s;
    }

    // Lists `folder`'s children with their icon location as the folder's
    // own extractor reports it (GIL flags: 0x2 not a file name, 0x10
    // don't cache, 0x20 per-instance).
    public static string tag = "x";
    public static string Run(string folder) {
        var sb = new StringBuilder();
        IntPtr abs; uint at;
        int hr = SHParseDisplayName(folder, IntPtr.Zero, out abs, 0, out at);
        if (hr != 0) return "parse " + hr.ToString("x");
        Guid iidSf = typeof(IShellFolder).GUID;
        IntPtr psf;
        hr = SHBindToObject(IntPtr.Zero, abs, IntPtr.Zero, ref iidSf, out psf);
        if (hr != 0) return "bind " + hr.ToString("x");
        var sf = (IShellFolder)Marshal.GetObjectForIUnknown(psf);
        IntPtr pe; sf.EnumObjects(IntPtr.Zero, 0x20 | 0x40 | 0x80, out pe);
        var en = (IEnumIDList)Marshal.GetObjectForIUnknown(pe);
        Guid iidEi = typeof(IExtractIconW).GUID;
        IntPtr child; uint got;
        while (en.Next(1, out child, out got) == 0 && got == 1) {
            IntPtr full = ILCombine(abs, child);
            IntPtr pei;
            int h = sf.GetUIObjectOf(IntPtr.Zero, 1, new[] { child }, ref iidEi, IntPtr.Zero, out pei);
            string loc = "no extractor " + h.ToString("x");
            if (h == 0) {
                var ei = (IExtractIconW)Marshal.GetObjectForIUnknown(pei);
                var file = new StringBuilder(520); int index; uint fl;
                int g = ei.GetIconLocation(0, file, 520, out index, out fl);
                loc = string.Format("{0},{1} flags 0x{2:x} hr {3:x}", file, index, fl, g);
                IntPtr large, small;
                int x = ei.Extract(file.ToString(), (uint)index, out large, out small, (16u << 16) | 48u);
                if (x == 0 && large != IntPtr.Zero) {
                    loc += " picture " + Pixels(large, Name(full).Replace(' ', '_') + "-" + tag);
                } else { loc += " extract hr " + x.ToString("x"); }
            }
            sb.AppendLine(Name(full) + ": " + loc);
        }
        return sb.ToString();
    }
}
