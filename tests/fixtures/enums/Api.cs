namespace EnumFixture {
    public enum Direction {
        Negative = -2,
        Zero = 0,
        Positive = 2,
        Alias = 2,
        Next
    }
    public enum SignedByte : sbyte { Minimum = -128, Zero = 0, Maximum = 127 }
    public enum UnsignedByte : byte { Zero = 0, Maximum = 255 }
    public enum SignedShort : short { Minimum = -32768, Maximum = 32767 }
    public enum UnsignedShort : ushort { Zero = 0, Maximum = 65535 }
    public enum SignedInt : int { Minimum = -2147483648, Maximum = 2147483647 }
    public enum UnsignedInt : uint { Zero = 0, Maximum = 4294967295U }
    public enum SignedLong : long { Minimum = -9223372036854775808L, Maximum = 9223372036854775807L }
    public enum UnsignedLong : ulong { Zero = 0, Maximum = 18446744073709551615UL }
    public enum Empty { }
    internal enum InternalCode { One = 1 }
    [System.Flags]
    public enum Bits {
        Read = 1 << 0, Write = Read << 1, Both = Read | Write,
        Future = Last, AfterFuture, Last = 1 << 3,
        Minus = ~0, Mask = (Both ^ Read) & 3,
        Precedence = 1 + 2 * 3, Shift = (1 << 31) >> 30
    }
    public enum ByteFlags : byte {
        High = 1 << 7, Low = 1, Both = High | Low, Invert = (~0) & 255,
        Arithmetic = 256 - 1
    }

    public static class Api {
        public static Direction Current = Direction.Positive;
        public static UnsignedLong ZeroField = 0;
        public static Direction Echo(Direction value) { return value; }
        public static Direction[] EchoArray(Direction[] value) { return value; }
        public static Direction Cast(int value) { return (Direction)value; }
        public static UnsignedLong EchoWide(UnsignedLong value) { return value; }
        public static UnsignedLong ZeroMethod() { return 1 - 1; }
        public static int Run() {
            Direction value = Direction.Positive;
            if (value == Direction.Positive) return (int)Echo(value);
            return -1;
        }
        public static int RoundTrip() { return (int)Echo((Direction)(-2)); }
        public static int FlagConstants() {
            return (int)Bits.AfterFuture + (int)ByteFlags.Both + (int)Bits.Precedence + (int)Bits.Shift;
        }
        public static int ZeroConversions() {
            UnsignedLong value = 0;
            value = 2 - 2;
            value = 0U;
            value = 0L;
            value = 0UL;
            UnsignedLong[] values = new UnsignedLong[1];
            values[0] = 0;
            if (value != 0) return -1;
            if (0 != value) return -2;
            if (values[0] != ZeroMethod()) return -3;
            if (EchoWide(0) != ZeroField) return -4;
            return 4;
        }
        public static int Arrays() {
            SignedByte[] a = new SignedByte[1]; a[0] = SignedByte.Minimum;
            UnsignedByte[] b = new UnsignedByte[1]; b[0] = UnsignedByte.Maximum;
            SignedShort[] c = new SignedShort[1]; c[0] = SignedShort.Minimum;
            UnsignedShort[] d = new UnsignedShort[1]; d[0] = UnsignedShort.Maximum;
            SignedInt[] e = new SignedInt[1]; e[0] = SignedInt.Minimum;
            UnsignedInt[] f = new UnsignedInt[1]; f[0] = UnsignedInt.Maximum;
            SignedLong[] g = new SignedLong[1]; g[0] = SignedLong.Minimum;
            UnsignedLong[] h = new UnsignedLong[1]; h[0] = UnsignedLong.Maximum;
            if (a[0] != SignedByte.Minimum) return -1;
            if (b[0] != UnsignedByte.Maximum) return -2;
            if (c[0] != SignedShort.Minimum) return -3;
            if (d[0] != UnsignedShort.Maximum) return -4;
            if (e[0] != SignedInt.Minimum) return -5;
            if (f[0] != UnsignedInt.Maximum) return -6;
            if (g[0] != SignedLong.Minimum) return -7;
            if (h[0] != UnsignedLong.Maximum) return -8;
            int count = 0;
            foreach (UnsignedLong value in h) {
                if (value == UnsignedLong.Maximum) count = count + 1;
            }
            return count + 8;
        }
    }
}
namespace EnumOne {
    public enum Shared { Value = 1 }
    public static class Holder {
        public static Shared Echo(Shared value) { return value; }
        public static int Run() { return (int)Echo(Shared.Value); }
    }
}
namespace EnumTwo {
    public enum Shared { Value = 2 }
    public static class OtherHolder {
        public static Shared Echo(Shared value) { return value; }
        public static int Run() { return (int)Echo(Shared.Value); }
    }
}
namespace EnumImported {
    using EnumOne;
    public static class ImportedHolder {
        public static Shared Echo(Shared value) { return value; }
        public static int Run() { return (int)Echo(Shared.Value); }
    }
}
namespace EnumAliased {
    using Chosen = EnumTwo.Shared;
    public static class AliasHolder {
        public static Chosen Echo(Chosen value) { return value; }
        public static int Run() { return (int)Echo(Chosen.Value); }
    }
}
