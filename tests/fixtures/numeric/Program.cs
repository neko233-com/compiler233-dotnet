namespace NumericFixture {
    public class Box {
        public float Field = 3;
        public float Value { get; set; }
    }
    public static class Runner {
        static int calls;
        static int NextInt() { calls = calls * 10 + 1; return 3; }
        static float NextFloat() { calls = calls * 10 + 2; return 2.5f; }
        static float Identity(float value) => value;
        public static int NanLessEqual() { float n = 0.0f / 0.0f; if (n <= 1.0f) return 1; return 0; }
        public static int NanGreaterEqual() { float n = 0.0f / 0.0f; if (n >= 1.0f) return 1; return 0; }
        public static int NormalCompare() { float n = 2.5f; if (n >= 2.5f && n <= 2.5f) return 1; return 0; }
        public static float Negation() => -1.25f;
        public static float ReturnInt() => 3;
        public static float Promotion() {
            int value = 3;
            float local = value;
            local += 2;
            float[] items = new float[1];
            items[0] = value;
            return Identity(value) + local + items[0];
        }
        public static float Fields() {
            Box box = new Box();
            box.Field = 4;
            box.Value = 5;
            return box.Field + box.Value;
        }
        public static float InitialField() { Box box = new Box(); return box.Field; }
        public static float MixedLeft() => 3 + 2.5f;
        public static float MixedRight() => 2.5f + 3;
        public static float Rounding() { int n = 16777217; float value = n; return value + 1.0f; }
        public static int Evaluation() { float value = NextInt() + NextFloat(); if (value == 5.5f) return calls; return -1; }
        public static float Conditional() { bool test = false; return test ? 1.5f : 2; }
        public static int MixedCompare() { if (2 <= 2.5f && 3.5f > 3) return 1; return 0; }
    }
}
