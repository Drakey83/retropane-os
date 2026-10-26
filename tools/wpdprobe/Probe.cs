// The R61 probe (2026-10-26): asks Windows, through its own portable-device
// API (read-only), which resources it offers on the Axim and on each of
// its storages, and reads each one back. Run from PowerShell 7:
// Add-Type -TypeDefinition (Get-Content -Raw Probe.cs); [Probe]::Run("<WPD device path>")

using System;
using System.Runtime.InteropServices;
using System.Text;

[StructLayout(LayoutKind.Sequential)]
public struct PROPERTYKEY { public Guid fmtid; public uint pid; }

[ComImport, Guid("625e2df8-6392-4cf0-9ad1-3cfa5f17775c"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPortableDevice {
    void Open([MarshalAs(UnmanagedType.LPWStr)] string id, IntPtr clientInfo);
    void SendCommand(uint flags, IntPtr p, out IntPtr r);
    void Content([MarshalAs(UnmanagedType.Interface)] out IPortableDeviceContent c);
    void Cancel();
    void Close();
}

[ComImport, Guid("6a96ed84-7c73-4480-9938-bf5af477d426"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPortableDeviceContent {
    void EnumObjects(uint flags, [MarshalAs(UnmanagedType.LPWStr)] string parent, IntPtr filter, [MarshalAs(UnmanagedType.Interface)] out IEnumPortableDeviceObjectIDs e);
    void Properties(out IntPtr p);
    void Transfer([MarshalAs(UnmanagedType.Interface)] out IPortableDeviceResources r);
}

[ComImport, Guid("10ece955-cf41-4728-bfa0-41eedf1bbf19"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IEnumPortableDeviceObjectIDs {
    [PreserveSig] int Next(uint n, [Out, MarshalAs(UnmanagedType.LPArray, ArraySubType = UnmanagedType.LPWStr, SizeParamIndex = 0)] string[] ids, out uint fetched);
}

[ComImport, Guid("fd8878ac-d841-4d17-891c-e6829cdb6934"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPortableDeviceResources {
    void GetSupportedResources([MarshalAs(UnmanagedType.LPWStr)] string id, [MarshalAs(UnmanagedType.Interface)] out IPortableDeviceKeyCollection keys);
    void GetResourceAttributes([MarshalAs(UnmanagedType.LPWStr)] string id, ref PROPERTYKEY key, out IntPtr values);
    [PreserveSig] int GetStream([MarshalAs(UnmanagedType.LPWStr)] string id, ref PROPERTYKEY key, uint mode, out uint optimal, out IntPtr s);
}

[ComImport, Guid("dada2357-e0ad-492e-98db-dd61c53ba353"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IPortableDeviceKeyCollection {
    void GetCount(out uint n);
    void GetAt(uint i, out PROPERTYKEY k);
}

public static class Probe {
    static readonly Guid ClsidDevice = new Guid("728a21c5-3d9e-48d7-9810-864848f0f404");
    static readonly Guid ClsidValues = new Guid("0c15d503-d017-47ce-9016-7b3f978721cc");

    [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int ReadFn(IntPtr self, byte[] buf, int cb, out int got);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int ReleaseFn(IntPtr self);
    static string Read(IPortableDeviceResources r, string id, PROPERTYKEY k) {
        uint opt; IntPtr s;
        int hr = r.GetStream(id, ref k, 0, out opt, out s);
        if (hr != 0) return string.Format("GetStream hr 0x{0:x8}", hr);
        IntPtr vt = Marshal.ReadIntPtr(s);
        var read = Marshal.GetDelegateForFunctionPointer<ReadFn>(Marshal.ReadIntPtr(vt, 3 * IntPtr.Size));
        var rel = Marshal.GetDelegateForFunctionPointer<ReleaseFn>(Marshal.ReadIntPtr(vt, 2 * IntPtr.Size));
        long total = 0; var buf = new byte[65536]; byte[] head = null;
        while (true) {
            int got; int rh = read(s, buf, buf.Length, out got);
            if (head == null && got > 0) { head = new byte[Math.Min(got, 8)]; Array.Copy(buf, head, head.Length); }
            if (got <= 0 || rh < 0) break;
            total += got;
        }
        rel(s);
        return string.Format("{0} bytes, starts {1}", total, BitConverter.ToString(head ?? new byte[0]));
    }

    public static string Run(string deviceId) {
        var sb = new StringBuilder();
        var dev = (IPortableDevice)Activator.CreateInstance(Type.GetTypeFromCLSID(ClsidDevice));
        object values = Activator.CreateInstance(Type.GetTypeFromCLSID(ClsidValues));
        IntPtr unk = Marshal.GetIUnknownForObject(values);
        Guid iidValues = new Guid("6848f6f2-3155-4f86-b6f5-263eeeab3143");
        IntPtr ivals; Marshal.QueryInterface(unk, in iidValues, out ivals);
        dev.Open(deviceId, ivals);
        IPortableDeviceContent content; dev.Content(out content);
        IPortableDeviceResources res; content.Transfer(out res);
        var ids = new System.Collections.Generic.List<string> { "DEVICE" };
        IEnumPortableDeviceObjectIDs e; content.EnumObjects(0, "DEVICE", IntPtr.Zero, out e);
        var one = new string[1]; uint got;
        while (e.Next(1, one, out got) == 0 && got == 1) ids.Add(one[0]);
        foreach (var id in ids) {
            sb.AppendLine("object " + id);
            IPortableDeviceKeyCollection keys;
            try { res.GetSupportedResources(id, out keys); }
            catch (Exception x) { sb.AppendLine("  resources: " + x.Message); continue; }
            uint n; keys.GetCount(out n);
            for (uint i = 0; i < n; i++) {
                PROPERTYKEY k; keys.GetAt(i, out k);
                sb.AppendLine(string.Format("  resource {0} {1}: {2}", k.fmtid, k.pid, Read(res, id, k)));
            }
        }
        dev.Close();
        return sb.ToString();
    }
}
