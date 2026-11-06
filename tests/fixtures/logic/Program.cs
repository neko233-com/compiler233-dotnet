namespace LogicFixture {
    public static class Runner {
        static int calls;
        static bool Record(int mark, bool value) { calls = calls * 10 + mark; return value; }
        static int Divide(int value) => 1 / value;
        static bool Bomb() => Divide(0) > 0;
        public static int OrTable() {
            int result = 0;
            if (false || false) result += 1;
            if (false || true) result += 2;
            if (true || false) result += 4;
            if (true || true) result += 8;
            return result;
        }
        public static int AndTable() {
            int result = 0;
            if (false && false) result += 1;
            if (false && true) result += 2;
            if (true && false) result += 4;
            if (true && true) result += 8;
            return result;
        }
        public static int OrSkip() { bool result = Record(1, true) || Record(2, false); return calls; }
        public static int OrEvaluate() { bool result = Record(1, false) || Record(2, false); return calls; }
        public static int AndSkip() { bool result = Record(1, false) && Record(2, true); return calls; }
        public static int AndEvaluate() { bool result = Record(1, true) && Record(2, false); return calls; }
        public static bool Nested() => false || (false && Bomb()) || (true && true);
        public static bool OrSkipException() => true || Bomb();
        public static bool AndSkipException() => false && Bomb();
        public static bool OrException() => false || Bomb();
        public static bool AndException() => true && Bomb();
    }
}
