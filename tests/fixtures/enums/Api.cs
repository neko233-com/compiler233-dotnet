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
    public enum UnsignedInt : uint { Zero = 0, One = 1, Two = 2, Maximum = 4294967295U }
    public enum SignedLong : long { Minimum = -9223372036854775808L, Zero = 0, Maximum = 9223372036854775807L }
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
    public class FlagBox {
        public Bits Field = Bits.Both;
        public Bits Property { get; set; }
    }
    public class CounterBox {
        public UnsignedByte Value;
        public int Gets;
        public int Sets;
        public UnsignedByte Property {
            get { Gets = Gets + 1; return Value; }
            set { Sets = Sets + 1; Value = value; }
        }
    }

    public static class Api {
        public static Direction Current = Direction.Positive;
        public static UnsignedLong ZeroField = 0;
        public static Bits FlagField = Bits.Read;
        public static int Calls = 0;
        public static UnsignedByte StaticCounter = UnsignedByte.Zero;
        public static UnsignedByte StaticProperty { get; set; }
        public static CounterBox SharedCounter = new CounterBox();
        public static Direction Echo(Direction value) { return value; }
        public static Direction[] EchoArray(Direction[] value) { return value; }
        public static Direction Cast(int value) { return (Direction)value; }
        public static UnsignedLong EchoWide(UnsignedLong value) { return value; }
        public static UnsignedLong ZeroMethod() { return 1 - 1; }
        public static bool TrueTick() { Calls = Calls + 1; return true; }
        public static bool FalseTick() { Calls = Calls + 10; return false; }
        public static CounterBox GetCounter() { Calls = Calls + 1; return SharedCounter; }
        public static Direction NextDirection(Direction value) { return ++value; }
        public static int NextAmount() { Calls = Calls + 1; return 1; }
        public static Direction NextValue() { Calls = Calls * 10 + 2; return Direction.Zero; }
        public static Direction AddDirection(Direction value, int amount) { return value + amount; }
        public static Direction AddFromLeft(int amount, Direction value) { return amount + value; }
        public static int Difference(Direction left, Direction right) { return left - right; }
        public static int SignedDifference(SignedInt left, SignedInt right) { return left - right; }
        public static int Run() {
            Direction value = Direction.Positive;
            if (value == Direction.Positive) return (int)Echo(value);
            return -1;
        }
        public static int RoundTrip() { return (int)Echo((Direction)(-2)); }
        public static int FlagConstants() {
            return (int)Bits.AfterFuture + (int)ByteFlags.Both + (int)Bits.Precedence + (int)Bits.Shift;
        }
        public static int FlagOperations() {
            Bits value = Bits.Read | Bits.Write;
            if (value != Bits.Both) return -1;
            value &= Bits.Read;
            value |= Bits.Write;
            value ^= Bits.Read;
            value |= 0;
            if (value != Bits.Write || (0 | value) != Bits.Write) return -2;
            if ((~Bits.Read & Bits.Both) != Bits.Write) return -3;
            if ((Bits.Read | Bits.Write & Bits.Read) != Bits.Read) return -4;
            if (~SignedByte.Minimum != SignedByte.Maximum) return -5;
            if (~UnsignedByte.Maximum != UnsignedByte.Zero) return -6;
            if (~SignedShort.Minimum != SignedShort.Maximum) return -7;
            if (~UnsignedShort.Maximum != UnsignedShort.Zero) return -8;
            if (~SignedInt.Minimum != SignedInt.Maximum) return -9;
            if (~UnsignedInt.Maximum != UnsignedInt.Zero) return -10;
            if (~SignedLong.Minimum != SignedLong.Maximum) return -11;
            if (~UnsignedLong.Maximum != UnsignedLong.Zero) return -12;
            if ((UnsignedLong.Maximum | 0) != UnsignedLong.Maximum || (0 | UnsignedLong.Maximum) != UnsignedLong.Maximum) return -15;
            UnsignedLong wide = UnsignedLong.Maximum;
            wide &= 0;
            wide |= UnsignedLong.Maximum;
            if (wide != UnsignedLong.Maximum) return -17;
            FlagBox box = new FlagBox();
            box.Property = Bits.Both;
            box.Field &= Bits.Read;
            box.Property ^= Bits.Read;
            FlagField |= Bits.Write;
            if (box.Field != Bits.Read || box.Property != Bits.Write || FlagField != Bits.Both) return -13;
            int integer = 1 | 2 & 4 ^ 8;
            bool boolean = true | false & false;
            if (integer != 9 || !boolean) return -14;
            Calls = 0;
            bool bothCalled = TrueTick() | FalseTick();
            if (!bothCalled || Calls != 11) return -16;
            return 42;
        }
        public static int FlagCase(Bits value) {
            switch (value) {
                case Bits.Read | Bits.Write: return 9;
                case ~Bits.Read: return 10;
                default: return 0;
            }
        }
        public static int FlagSwitch() { return FlagCase(Bits.Both) + FlagCase(~Bits.Read); }
        public static int IncrementEnums() {
            SignedByte a = SignedByte.Maximum; SignedByte oldA = a++;
            if (oldA != SignedByte.Maximum || a != SignedByte.Minimum) return -1;
            UnsignedByte b = UnsignedByte.Maximum;
            if (++b != UnsignedByte.Zero || b != UnsignedByte.Zero) return -2;
            SignedShort c = SignedShort.Maximum;
            if (c++ != SignedShort.Maximum || c != SignedShort.Minimum) return -3;
            UnsignedShort d = UnsignedShort.Maximum;
            if (++d != UnsignedShort.Zero) return -4;
            SignedInt e = SignedInt.Maximum;
            if (e++ != SignedInt.Maximum || e != SignedInt.Minimum) return -5;
            UnsignedInt f = UnsignedInt.Maximum;
            if (++f != UnsignedInt.Zero) return -6;
            SignedLong g = SignedLong.Maximum;
            if (g++ != SignedLong.Maximum || g != SignedLong.Minimum) return -7;
            UnsignedLong h = UnsignedLong.Zero;
            if (h-- != UnsignedLong.Zero || h != UnsignedLong.Maximum) return -8;
            StaticCounter = UnsignedByte.Maximum;
            if (Api.StaticCounter++ != UnsignedByte.Maximum || StaticCounter != UnsignedByte.Zero) return -9;
            StaticProperty = UnsignedByte.Maximum;
            if (++Api.StaticProperty != UnsignedByte.Zero || StaticProperty != UnsignedByte.Zero) return -15;
            if ((int)NextDirection(Direction.Zero) != 1) return -10;
            SharedCounter.Value = UnsignedByte.Maximum;
            SharedCounter.Gets = 0;
            SharedCounter.Sets = 0;
            Calls = 0;
            UnsignedByte old = GetCounter().Property++;
            if (old != UnsignedByte.Maximum || SharedCounter.Value != UnsignedByte.Zero) return -11;
            if (Calls != 1 || SharedCounter.Gets != 1 || SharedCounter.Sets != 1) return -12;
            Calls = 0;
            int previousGets = GetCounter().Gets++;
            if (previousGets != 1 || SharedCounter.Gets != 2 || Calls != 1) return -14;
            SharedCounter.Value = UnsignedByte.Maximum;
            if (++SharedCounter.Value != UnsignedByte.Zero) return -13;
            UnsignedByte[] counters = new UnsignedByte[1];
            counters[0] = UnsignedByte.Maximum;
            int index = 0;
            UnsignedByte beforeArray = counters[index++]++;
            if (index != 1 || beforeArray != UnsignedByte.Maximum || counters[0] != UnsignedByte.Zero) return -16;
            if ((int)(++counters[0]) != 1) return -17;
            int[] integers = new int[1];
            integers[0] = 4;
            if (integers[0]++ != 4 || integers[0] != 5) return -18;
            return 44;
        }
        public static int IncrementExceptions() {
            int seen = 0;
            int index = 0;
            int[] missing = null;
            try { missing[index++]++; } catch (System.NullReferenceException) { seen = seen + 1; }
            if (index != 1) return -1;
            int[] empty = new int[0];
            try { ++empty[0]; } catch (System.IndexOutOfRangeException) { seen = seen + 1; }
            return seen;
        }
        public static int EnumArithmetic() {
            Direction x = AddDirection(Direction.Positive, 1);
            if (x != Direction.Next || AddFromLeft(1, Direction.Positive) != Direction.Next) return -1;
            if (Difference(Direction.Positive, Direction.Negative) != 4) return -2;
            if (SignedDifference(SignedInt.Maximum, SignedInt.Minimum) != -1) return -16;
            if ((0 - Direction.Positive) != -2 || (Direction.Positive - 0U) != 2) return -3;
            Direction same = Direction.Positive - 0;
            if (same != Direction.Positive) return -4;
            SignedByte a = SignedByte.Maximum; a = a + 1;
            UnsignedByte b = UnsignedByte.Maximum; b = b + 1;
            SignedShort c = SignedShort.Maximum; c = c + 1;
            UnsignedShort d = UnsignedShort.Maximum; d = d + 1;
            SignedInt e = SignedInt.Maximum; e = e + 1;
            UnsignedInt f = UnsignedInt.Maximum; f = f + 1;
            SignedLong g = SignedLong.Maximum; g = g + 1;
            UnsignedLong h = UnsignedLong.Maximum; h = h + 1;
            if (a != SignedByte.Minimum || b != UnsignedByte.Zero) return -5;
            if (c != SignedShort.Minimum || d != UnsignedShort.Zero) return -6;
            if (e != SignedInt.Minimum || f != UnsignedInt.Zero) return -7;
            if (g != SignedLong.Minimum || h != UnsignedLong.Zero) return -8;
            b = b - 1;
            if (b != UnsignedByte.Maximum) return -9;
            int negative = -1;
            g = SignedLong.Zero + negative;
            if ((int)g != -1) return -10;
            Calls = 0;
            Direction ordered = NextAmount() + NextValue();
            if ((int)ordered != 1 || Calls != 12) return -11;
            a = SignedByte.Maximum; a += 1;
            if (a != SignedByte.Minimum) return -12;
            b = UnsignedByte.Zero; b -= 1;
            if (b != UnsignedByte.Maximum) return -13;
            h = UnsignedLong.Maximum; h += 1;
            if (h != UnsignedLong.Zero) return -14;
            FlagBox box = new FlagBox();
            box.Property = Bits.Read;
            box.Field -= 1;
            box.Property += 1;
            if (box.Field != Bits.Write || box.Property != Bits.Write) return -15;
            SharedCounter.Value = UnsignedByte.Maximum;
            Calls = 0;
            GetCounter().Value += 1;
            if (SharedCounter.Value != UnsignedByte.Zero || Calls != 1) return -17;
            return 45;
        }
        public static int Classify(Direction value) {
            switch (value) {
                case Direction.Negative: return 3;
                case (1 & 0): return 4;
                case Direction.Positive: return 5;
                case Direction.Positive + 1: return 7;
                default: return 6;
            }
        }
        public static int SwitchSignedByte(SignedByte value) { switch (value) { case SignedByte.Minimum: return 1; default: return 0; } }
        public static int SwitchUnsignedByte(UnsignedByte value) { switch (value) { case UnsignedByte.Maximum: return 2; default: return 0; } }
        public static int SwitchSignedShort(SignedShort value) { switch (value) { case SignedShort.Minimum: return 3; default: return 0; } }
        public static int SwitchUnsignedShort(UnsignedShort value) { switch (value) { case UnsignedShort.Maximum: return 4; default: return 0; } }
        public static int SwitchSignedInt(SignedInt value) { switch (value) { case SignedInt.Minimum: return 5; default: return 0; } }
        public static int SwitchUnsignedInt(UnsignedInt value) { switch (value) { case UnsignedInt.Maximum: return 6; default: return 0; } }
        public static int SwitchSignedLong(SignedLong value) { switch (value) { case SignedLong.Minimum: return 7; default: return 0; } }
        public static int SwitchUnsignedLong(UnsignedLong value) { switch (value) { case UnsignedLong.Maximum: return 8; default: return 0; } }
        public static int SwitchEnums() {
            return Classify(Direction.Negative) + Classify(Direction.Zero) + Classify(Direction.Positive)
                + Classify(Direction.Next) + Classify((Direction)8) + Classify((Direction)(-1))
                + Classify((Direction)1) + Classify((Direction)4);
        }
        public static int ClassifyUnsigned(UnsignedInt value) {
            switch (value) {
                case UnsignedInt.Maximum: return 1;
                case UnsignedInt.Zero: return 2;
                case UnsignedInt.One: return 3;
                case UnsignedInt.Two: return 4;
                default: return 5;
            }
        }
        public static int SwitchUnsignedDense() {
            return ClassifyUnsigned(UnsignedInt.Maximum) + ClassifyUnsigned(UnsignedInt.Zero)
                + ClassifyUnsigned(UnsignedInt.One) + ClassifyUnsigned(UnsignedInt.Two)
                + ClassifyUnsigned((UnsignedInt)3);
        }
        public static int NoDefault(Direction value) {
            int result = 7;
            switch (value) {
                case Direction.Positive: result = 2; break;
                case Direction.Zero:
                case Direction.Negative: result = 4; break;
            }
            return result;
        }
        public static int SwitchNoDefault() {
            return NoDefault((Direction)8) + NoDefault(Direction.Positive) + NoDefault(Direction.Negative);
        }
        public static int SwitchAll() {
            return SwitchSignedByte(SignedByte.Minimum) + SwitchUnsignedByte(UnsignedByte.Maximum)
                + SwitchSignedShort(SignedShort.Minimum) + SwitchUnsignedShort(UnsignedShort.Maximum)
                + SwitchSignedInt(SignedInt.Minimum) + SwitchUnsignedInt(UnsignedInt.Maximum)
                + SwitchSignedLong(SignedLong.Minimum) + SwitchUnsignedLong(UnsignedLong.Maximum);
        }
        public static int ZeroConversions() {
            UnsignedLong value = 0;
            value = 2 - 2;
            value = 0U;
            value = 0L;
            value = 0UL;
            value = ~(-1);
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
