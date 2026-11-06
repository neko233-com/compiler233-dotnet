namespace StaticFixture {
    public enum FieldCode { None = 0, A = 1, B = 2, Both = 3 }
    public class Box { public int Number; }
    public static class FieldAccess {
        public static int Number = 1;
        public static FieldCode Code = FieldCode.None;
        public static string Text = "initial";
        public static bool Enabled = false;
        public static float Rate = 1f;
        public static object Value = null;
        public static int[] Numbers = null;
    }
    public static class OrderTracker {
        public static int Trace;
        public static void Reset() { Trace = 0; }
        public static void Mark(int value) { Trace = Trace * 10 + value; }
        public static int Next() { Mark(1); return 1; }
        public static int Read() { return Trace; }
    }
    public static class LateStore {
        public static int Value;
        static LateStore() { OrderTracker.Mark(2); }
    }
    public static class EarlyRead {
        public static int Value;
        static EarlyRead() { OrderTracker.Mark(2); }
    }
    public static class Operations {
        public static int Trace = Digit(1);
        public static int Value = Digit(2);
        static Operations() { Trace = Trace * 10 + 3; Value += 5; }
        private static int Digit(int value) { Trace = Trace * 10 + value; return value; }
        public static int Run() { return Trace * 10 + Value; }
    }
    public static class Plain {
        public static int Add(int value) { return value + 1; }
    }
    public class Ordinary {
        public static int Steps = Digit(4);
        static Ordinary() { Steps = Steps * 10 + 5; }
        private static int Digit(int value) { Steps = Steps * 10 + value; return Steps; }
        public static void Touch() { Steps += 10; }
        public static void Increase(int value) { Steps += value; }
        public static int Read() { return Steps; }
    }
    public static class NoFields {
        static NoFields() { Ordinary.Increase(100); }
        public static int Read() { return Ordinary.Read(); }
    }
    public static class ExpressionBody {
        static ExpressionBody() => Ordinary.Touch();
        public static int Read() { return Ordinary.Read(); }
    }
    public static class Failing {
        static Failing() { Ordinary.Touch(); throw new System.InvalidOperationException(); }
        public static int Read() { return 1; }
    }
    public interface Marker {
        static Marker() { Ordinary.Touch(); }
        void M();
    }
    public static class Runner {
        public static int Shadow() {
            Box FieldAccess = new Box();
            FieldAccess.Number = 8;
            return FieldAccess.Number;
        }
        public static int Run() {
            if (Operations.Run() != 1237) return -1;
            if (Ordinary.Read() != 45) return -2;
            if (NoFields.Read() != 145) return -3;
            if (ExpressionBody.Read() != 155) return -4;
            if (Plain.Add(3) != 4) return -5;
            FieldAccess.Number = 2;
            FieldAccess.Number += 3;
            int previous = FieldAccess.Number++;
            FieldAccess.Code = FieldCode.A;
            FieldAccess.Code |= FieldCode.B;
            FieldAccess.Enabled = true;
            FieldAccess.Enabled &= true;
            FieldAccess.Rate = 2f;
            FieldAccess.Text = "value";
            FieldAccess.Numbers = new int[1];
            FieldAccess.Numbers[0] = 9;
            if (previous != 5 || FieldAccess.Number != 6 || FieldAccess.Code != FieldCode.Both) return -6;
            if (!FieldAccess.Enabled || FieldAccess.Rate != 2f || FieldAccess.Text != "value" || FieldAccess.Value != null || FieldAccess.Numbers[0] != 9) return -7;
            if (Shadow() != 8) return -8;
            OrderTracker.Reset();
            LateStore.Value = OrderTracker.Next();
            if (OrderTracker.Read() != 12) return -9;
            OrderTracker.Reset();
            EarlyRead.Value += OrderTracker.Next();
            if (OrderTracker.Read() != 21) return -10;
            return 49;
        }
    }
}
