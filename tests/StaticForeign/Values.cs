namespace ForeignFieldFixture {
    public enum Status : long { Zero = 0, One = 1, Two = 2, Both = 3, Max = long.MaxValue }
    public static class Values {
        public static int Counter = 2;
        public static Status Mode = Status.Zero;
        public static float Rate = 1f;
        public static bool Ready = false;
        public static string Text = "start";
        public static object Value = null;
        public static int[] Numbers = null;
        public static Status[] Codes = null;
        public static readonly int ReadOnly = 6;
        public const int Literal = 9;
        public const int Max = int.MaxValue;
        public const int Zero = 0;
        public const bool Flag = true;
        public const float Ratio = 1.5f;
        public const string Label = "foreign";
        public const string NoText = null;
        public const Status ConstantStatus = Status.Two;
        public const Status MaximumStatus = Status.Max;
        private static int Hidden = 1;
        public static int ReadHidden() { return Hidden; }
    }
    public class Instance { public int Number; }
}
namespace OtherForeign { public static class Values { public static int Counter; } }
