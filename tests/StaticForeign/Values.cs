namespace ForeignFieldFixture {
    public enum Status : long { Zero = 0, One = 1, Two = 2, Both = 3 }
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
        private static int Hidden = 1;
        public static int ReadHidden() { return Hidden; }
    }
    public class Instance { public int Number; }
}
namespace OtherForeign { public static class Values { public static int Counter; } }
