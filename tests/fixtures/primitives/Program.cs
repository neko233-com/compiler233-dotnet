namespace PrimitiveFixture {
    public static class Runner {
        static int calls;
        static int Next() { calls += 1; return 255; }
        public static int Once() { string text = Next().ToString("X"); if (text == "FF") return calls; return -1; }
        public static string Integer() { int value = -17; return value.ToString("D4"); }
        public static string NullFormat() { int value = 17; string format = null; return value.ToString(format); }
        public static string Single() { float value = 1.5f; return value.ToString(); }
        public static string Boolean() { bool value = true; return value.ToString(); }
        public static int IntegerHash() { int value = 213; return value.GetHashCode(); }
        public static int StringHash() { string value = "compiler233"; return value.GetHashCode(); }
        public static string StringValue() { string value = "hello"; return value.ToString(); }
        public static string NullString() { string value = null; return value.ToString(); }
        public static string NullObject() { object value = null; return value.ToString(); }
    }
}
