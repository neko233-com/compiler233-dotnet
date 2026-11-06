namespace ArrayFixture {
    public static class Runner {
        public static float FloatRead() {
            float[] values = new float[2];
            values[0] = 1.5f;
            values[1] = 2.25f;
            return values[0] + values[1];
        }
        public static float FloatEach() {
            float[] values = new float[2];
            values[0] = 1.5f;
            values[1] = 2.25f;
            float sum = 0.0f;
            foreach (float value in values) { sum += value; }
            return sum;
        }
        public static int BoolRead() {
            bool[] values = new bool[3];
            values[0] = true;
            values[1] = false;
            values[2] = true;
            if (values[0] && !values[1] && values[2]) return 1;
            return 0;
        }
        public static int BoolEach() {
            bool[] values = new bool[3];
            values[0] = true;
            values[2] = true;
            int count = 0;
            foreach (bool value in values) { if (value) count += 1; }
            return count;
        }
        public static int IntEach() {
            int[] values = new int[2];
            values[0] = 3;
            values[1] = 4;
            int sum = 0;
            foreach (int value in values) { sum += value; }
            return sum;
        }
        public static string ReferenceRead() {
            string[] values = new string[2];
            values[0] = "hello";
            values[1] = null;
            return values[0] + values[1];
        }
        public static string ObjectRead() {
            object[] values = new object[2];
            values[0] = "hello";
            values[1] = null;
            return values[0].ToString();
        }
        public static int ReferenceEach() {
            string[] values = new string[2];
            values[0] = "hello";
            int count = 0;
            foreach (string value in values) { if (value != null) count += 1; }
            return count;
        }
        public static int Bounds() { int[] values = new int[1]; return values[2]; }
        public static int WriteBounds() { bool[] values = new bool[1]; values[2] = true; return 0; }
        public static int NullArray() { float[] values = null; return (int)values[0]; }
    }
}
