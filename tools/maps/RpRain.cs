// The dock relay's rain frame (R184 step 2, run today-0055): RainViewer's
// nine tiles for the X51v's view stitched, cropped to the view and encoded
// as RPRN (rp_weather::rprn, the format's reference and parser): a palette
// and each row's runs. Loaded once by the relay (Add-Type, Windows
// PowerShell 5.1: C# 5, System.Drawing, System.Net.Http). One answer a
// frame instead of nine tiles, no PNG decoding on the device.
//
// Fetching (the third X51v run: one tile after another took ~1.5 s a
// frame): a frame's nine tiles are fetched at once, and once the device
// asks for one frame of a view, every frame of the list is fetched for that
// view in the background, so each answer is ready when asked for and the
// dock's transfer is all the device waits for.
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Net;
using System.Net.Http;
using System.Runtime.InteropServices;
using System.Threading.Tasks;

public static class RpRain
{
    public const int W = 480;
    public const int H = 462;

    static readonly HttpClient Http = MakeClient();
    static readonly ConcurrentDictionary<string, Task<byte[]>> Frames =
        new ConcurrentDictionary<string, Task<byte[]>>();

    static HttpClient MakeClient()
    {
        ServicePointManager.SecurityProtocol |= SecurityProtocolType.Tls12;
        ServicePointManager.DefaultConnectionLimit = 16;
        return new HttpClient { Timeout = TimeSpan.FromSeconds(20) };
    }

    // Frame `path` for the view whose top-left tile is (tx, ty) at zoom z,
    // the view starting (ox, oy) inside it: fetched once, then kept until
    // the list moves on (Keep). The task's result is the RPRN file, or null
    // with more than 255 colours; it faults if a tile can't be fetched.
    public static Task<byte[]> Frame(string host, string path, int z, int tx, int ty, int ox, int oy, string ua)
    {
        string key = path + "|" + z + "|" + tx + "|" + ty + "|" + ox + "|" + oy;
        return Frames.GetOrAdd(key, k => Fetch(host, path, z, tx, ty, ox, oy, ua));
    }

    // Every frame of the list for that view, fetched in the background.
    public static void Prefetch(string host, string[] paths, int z, int tx, int ty, int ox, int oy, string ua)
    {
        foreach (string p in paths) Frame(host, p, z, tx, ty, ox, oy, ua);
    }

    // Frame `path` forgotten (its fetch failed: asked again, fetched again).
    public static void Drop(string path)
    {
        foreach (string k in Frames.Keys)
        {
            if (k.Substring(0, k.IndexOf('|')) == path)
            {
                Task<byte[]> gone;
                Frames.TryRemove(k, out gone);
            }
        }
    }

    // Frames no longer listed are forgotten (RainViewer's list moved on).
    public static void Keep(string[] paths)
    {
        HashSet<string> keep = new HashSet<string>(paths);
        foreach (string k in Frames.Keys)
        {
            if (!keep.Contains(k.Substring(0, k.IndexOf('|'))))
            {
                Task<byte[]> gone;
                Frames.TryRemove(k, out gone);
            }
        }
    }

    static async Task<byte[]> Fetch(string host, string path, int z, int tx, int ty, int ox, int oy, string ua)
    {
        int n = 1 << z;
        Task<byte[]>[] tiles = new Task<byte[]>[9];
        for (int k = 0; k < 9; k++)
        {
            int x = ((tx + k % 3) % n + n) % n;
            int y = ty + k / 3;
            if (y < 0 || y >= n) continue;
            string url = host + path + "/256/" + z + "/" + x + "/" + y + "/2/1_1.png";
            tiles[k] = Get(url, ua);
        }
        byte[][] got = new byte[9][];
        for (int k = 0; k < 9; k++)
        {
            if (tiles[k] != null) got[k] = await tiles[k].ConfigureAwait(false);
        }
        return Encode(got, ox, oy);
    }

    static async Task<byte[]> Get(string url, string ua)
    {
        using (HttpRequestMessage req = new HttpRequestMessage(HttpMethod.Get, url))
        {
            req.Headers.TryAddWithoutValidation("User-Agent", ua);
            using (HttpResponseMessage r = await Http.SendAsync(req).ConfigureAwait(false))
            {
                r.EnsureSuccessStatusCode();
                return await r.Content.ReadAsByteArrayAsync().ConfigureAwait(false);
            }
        }
    }

    // `tiles`: the 3 x 3 grid row by row, each a PNG (null: none, past the
    // poles); the view's top left at (ox, oy) in the grid. Null if the
    // frame holds more than 255 colours.
    public static byte[] Encode(byte[][] tiles, int ox, int oy)
    {
        int[] grid = new int[768 * 768];
        for (int k = 0; k < 9; k++)
        {
            if (tiles[k] == null) continue;
            using (MemoryStream ms = new MemoryStream(tiles[k]))
            using (Bitmap src = new Bitmap(ms))
            using (Bitmap bmp = src.Clone(new Rectangle(0, 0, 256, 256), PixelFormat.Format32bppArgb))
            {
                BitmapData d = bmp.LockBits(new Rectangle(0, 0, 256, 256), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
                int[] row = new int[256];
                for (int y = 0; y < 256; y++)
                {
                    Marshal.Copy(IntPtr.Add(d.Scan0, y * d.Stride), row, 0, 256);
                    Array.Copy(row, 0, grid, ((k / 3) * 256 + y) * 768 + (k % 3) * 256, 256);
                }
                bmp.UnlockBits(d);
            }
        }
        List<uint> pal = new List<uint>();
        Dictionary<uint, int> index = new Dictionary<uint, int>();
        MemoryStream runs = new MemoryStream();
        for (int y = 0; y < H; y++)
        {
            int at = (oy + y) * 768 + ox;
            int x = 0;
            while (x < W)
            {
                uint c = Clear((uint)grid[at + x]);
                int len = 1;
                while (x + len < W && len < 255 && Clear((uint)grid[at + x + len]) == c) len++;
                int idx = 0;
                if (c != 0 && !index.TryGetValue(c, out idx))
                {
                    pal.Add(c);
                    idx = pal.Count;
                    index[c] = idx;
                }
                if (idx > 255) return null;
                runs.WriteByte((byte)len);
                runs.WriteByte((byte)idx);
                x += len;
            }
        }
        if (pal.Count == 0) pal.Add(0);
        MemoryStream f = new MemoryStream();
        f.Write(new byte[] { (byte)'R', (byte)'P', (byte)'R', (byte)'N', 1, W & 0xFF, W >> 8, H & 0xFF, H >> 8, (byte)pal.Count }, 0, 10);
        foreach (uint c in pal) f.Write(BitConverter.GetBytes(c), 0, 4);
        runs.WriteTo(f);
        return f.ToArray();
    }

    // No rain: alpha 0, whatever its colour.
    static uint Clear(uint c)
    {
        return (c >> 24) == 0 ? 0u : c;
    }
}
