using EnumFixture;
using Picked = EnumFixture.Direction;

public static class ExternalConsumer {
    public static Direction Echo(Direction value) { return value; }
    public static Picked EchoAlias(Picked value) { return value; }
    public static Direction[] EchoArray(Direction[] value) { return value; }
    public static SignedByte EchoSignedByte(SignedByte value) { return value; }
    public static UnsignedByte EchoUnsignedByte(UnsignedByte value) { return value; }
    public static SignedShort EchoSignedShort(SignedShort value) { return value; }
    public static UnsignedShort EchoUnsignedShort(UnsignedShort value) { return value; }
    public static SignedInt EchoSignedInt(SignedInt value) { return value; }
    public static UnsignedInt EchoUnsignedInt(UnsignedInt value) { return value; }
    public static SignedLong EchoSignedLong(SignedLong value) { return value; }
    public static UnsignedLong EchoUnsignedLong(UnsignedLong value) { return value; }

    public static int Run() {
        Direction[] values = new Direction[1];
        values[0] = Direction.Positive;
        SignedByte[] a = new SignedByte[1]; a[0] = SignedByte.Minimum;
        UnsignedByte[] b = new UnsignedByte[1]; b[0] = UnsignedByte.Maximum;
        SignedShort[] c = new SignedShort[1]; c[0] = SignedShort.Minimum;
        UnsignedShort[] d = new UnsignedShort[1]; d[0] = UnsignedShort.Maximum;
        SignedInt[] e = new SignedInt[1]; e[0] = SignedInt.Minimum;
        UnsignedInt[] f = new UnsignedInt[1]; f[0] = UnsignedInt.Maximum;
        SignedLong[] g = new SignedLong[1]; g[0] = SignedLong.Minimum;
        UnsignedLong[] h = new UnsignedLong[1]; h[0] = UnsignedLong.Maximum;
        if (a[0] != EchoSignedByte(SignedByte.Minimum)) return -3;
        if (b[0] != EchoUnsignedByte(UnsignedByte.Maximum)) return -4;
        if (c[0] != EchoSignedShort(SignedShort.Minimum)) return -5;
        if (d[0] != EchoUnsignedShort(UnsignedShort.Maximum)) return -6;
        if (e[0] != EchoSignedInt(SignedInt.Minimum)) return -7;
        if (f[0] != EchoUnsignedInt(UnsignedInt.Maximum)) return -8;
        if (g[0] != EchoSignedLong(SignedLong.Minimum)) return -9;
        if (h[0] != EchoUnsignedLong(UnsignedLong.Maximum)) return -10;
        if (EchoAlias(Picked.Positive) != Direction.Positive) return -11;
        if ((Echo(Direction.Positive) + 1) != Direction.Next) return -15;
        UnsignedLong wrapped = UnsignedLong.Maximum;
        wrapped += 1;
        if (wrapped != UnsignedLong.Zero) return -16;
        object boxed = UnsignedLong.Maximum;
        if ((UnsignedLong)boxed != UnsignedLong.Maximum || Direction.Positive.ToString() != "Positive" || Direction.Positive.ToString("D") != "2") return -17;
        if ((UnsignedLong.Maximum ^ UnsignedLong.Maximum) != UnsignedLong.Zero) return -13;
        h[0] = UnsignedLong.Zero;
        if (h[0]-- != UnsignedLong.Zero || h[0] != UnsignedLong.Maximum) return -14;
        switch (h[0]) { case UnsignedLong.Maximum: break; default: return -12; }
        if (Api.Echo(values[0]) != Direction.Positive) return -2;
        return (int)Echo(EchoArray(values)[0]) + (int)UnsignedByte.Maximum;
    }
}
