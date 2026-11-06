// The dock relay's rain frame (R184 step 2, run today-0055): RainViewer's
// nine tiles for the X51v's view stitched, cropped to the view and encoded
// as RPRN (rp_weather::rprn, the format's reference and parser): a palette
// and each row's runs. Loaded once by the relay (Add-Type, Windows
// PowerShell 5.1: C# 5, System.Drawing). One answer a frame instead of
// nine tiles, no PNG decoding on the device.
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;

public static class RpRain
{
    public const int W = 480;
    public const int H = 462;

    // `tiles`: the 3 x 3 grid row by row, each a PNG (null: none, past the
    // poles or not fetched); the view's top left at (ox, oy) in the grid.
    // Null if the frame holds more than 255 colours.
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
