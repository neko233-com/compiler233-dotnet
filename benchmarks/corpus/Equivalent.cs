namespace CompilerBench {
    public static class Runner {
        static int Choose(int value) {
            switch (value) {
                case -1: return 7;
                case 0: return 8;
                case 1: return 9;
                case 2: return 10;
                default: return 11;
            }
        }
        public static int Arithmetic() {
            int value = 17;
            return (value * 3 - 4) / 2;
        }
        public static int Loops() {
            int value = 0; int total = 0;
            do { value += 1; if (value == 2) continue; total += value; } while (value < 4);
            for (int index = 0; index < 3; index++) total += index;
            return total;
        }
        public static int Dispatch() { return Choose(-1) * 100 + Choose(1) * 10 + Choose(7); }
        public static string Text() { string prefix = "al"; return prefix + "\u0070ha"; }
        public static int Arrays() {
            int[] values = new int[3];
            values[0] = 3; values[1] = 4; values[2] = 5;
            int total = 0;
            foreach (int value in values) total += value;
            return total;
        }
    }
}
