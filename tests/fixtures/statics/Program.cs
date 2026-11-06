namespace StaticFixture {
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
        public static int Run() {
            if (Operations.Run() != 1237) return -1;
            if (Ordinary.Read() != 45) return -2;
            if (NoFields.Read() != 145) return -3;
            if (ExpressionBody.Read() != 155) return -4;
            if (Plain.Add(3) != 4) return -5;
            return 49;
        }
    }
}
