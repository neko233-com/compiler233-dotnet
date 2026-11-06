namespace ConstantFixture {
    public static class Values {
        public const int Answer = 6 * 7;
        public const int Negative = -10;
        public const int Zero = default(int);
        public const int Max = int.MaxValue;
        public const bool Enabled = true;
        public const bool Disabled = default(bool);
        public const string Text = "常量✓";
        public const string Empty = "";
        public const string Missing = null;
        public const object NullObject = null;
        public static int OwnValue() { return Answer + Negative; }
    }
    public static class OnlyConstants { public const int Number = 5; }
    public interface IConstants { const int Seven = 7; }
    public static class Runner {
        public static int Run() {
            if (Values.Answer != 42 || Values.OwnValue() != 32 || Values.Zero != 0 || IConstants.Seven != 7) return -1;
            if (!Values.Enabled || Values.Disabled || Values.Text != "常量✓" || Values.Empty != "") return -2;
            if (Values.Missing != null || Values.NullObject != null) return -3;
            switch (42) { case Values.Answer: break; default: return -4; }
            switch (1) { case Values.Zero + 1: break; default: return -5; }
            switch (true) { case Values.Enabled: break; default: return -6; }
            switch ("常量✓") { case Values.Text: break; default: return -7; }
            switch (default(string)) { case Values.Missing: break; default: return -8; }
            return Values.Answer;
        }
    }
}
