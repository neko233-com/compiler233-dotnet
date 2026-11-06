namespace LoopFixture {
    public static class Runner {
        static int checks;
        static bool Check(int value) { checks += 1; return value < 3; }
        static bool UnexpectedCondition() { checks += 100; return true; }

        public static int ExecuteOnce() {
            int count = 0;
            do { count += 1; } while (false);
            return count;
        }
        public static int ContinueTestsCondition() {
            int count = 0;
            checks = 0;
            do {
                count += 1;
                if (count < 3) continue;
                checks += 10;
            } while (Check(count));
            return count * 100 + checks;
        }
        public static int BreakSkipsCondition() {
            checks = 0;
            do { checks += 1; break; } while (UnexpectedCondition());
            return checks;
        }
        public static int ContinueAndBreakThroughFinally() {
            int count = 0;
            int seen = 0;
            checks = 0;
            do {
                count += 1;
                try { if (count == 1) continue; break; }
                finally { seen = seen * 10 + count; }
            } while (Check(count));
            return seen * 100 + checks * 10 + count;
        }
        public static int LoopInsideFinally() {
            int value = 0;
            try { value = 1; }
            finally { do { value += 1; } while (value < 3); }
            return value;
        }
        public static int ScopedSiblings() {
            int result = 0;
            do { int value = 4; result += value; } while (false);
            do { int value = 3; result += value; } while (false);
            return result;
        }
        public static int NestedScopedSiblings() {
            int result = 0;
            do {
                if (true) { int value = 1; result += value; }
                if (true) { int value = 2; result += value; }
                for (int index = 0; index < 2; index++) { result += index; }
                for (int index = 0; index < 2; index++) { result += index; }
            } while (false);
            return result;
        }
        public static int EarlierSiblingScope() {
            int result = 0;
            if (true) { int value = 4; result += value; }
            do { int value = 3; result += value; } while (false);
            return result;
        }
        public static int NestedLoops() {
            int outer = 0; int total = 0;
            do {
                outer += 1;
                int inner = 0;
                while (inner < 3) { inner += 1; if (inner == 2) break; total += outer; }
            } while (outer < 3);
            return total;
        }
        public static int SingleAndEmptyBody() {
            int value = 0;
            do value += 1; while (value < 2);
            do ; while (false);
            return value;
        }
    }
}
