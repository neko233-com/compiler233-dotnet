using System;
namespace SwitchFixture {
    public static class Runner {
        static int state;
        static int Probe() { state += 1; return 42; }
        static int Select(int value) {
            switch (value) {
                case -1: return 7;
                case 0:
                case 1: return 5;
                default: return 9;
            }
        }
        static int SelectString(string value) {
            switch (value) {
                case "alpha": return 1;
                case "beta": return 2;
                case null: return 3;
                default: return 4;
            }
        }
        static int SelectEscaped(string value) {
            switch (value) {
                case "\u0061": return 1;
                case "\x41B": return 2;
                case "\uD83D\uDE00": return 3;
                case "\U0001F642": return 4;
                case "\U0000D800\U0000DC00": return 5;
                case "\a\b\f\v": return 6;
                default: return 9;
            }
        }
        static int SelectBool(bool value) {
            switch (value) { case true: return 1; case false: return 2; }
            return 3;
        }
        static int Dense(int value) {
            switch (value) {
                case -2: return 1;
                case -1: return 2;
                case 0: return 3;
                case 1: return 4;
                case 3: return 5;
                default: return 9;
            }
        }
        static int HexAndBinary(int value) {
            switch (value) { case 0x1e: return 1; case 0xFF: return 2; case 0b1010: return 3; default: return 9; }
        }
        static int DenseLow(int value) {
            switch (value) {
                case -2147483647: return 1;
                case -2147483646: return 2;
                case -2147483645: return 3;
                case -2147483644: return 4;
                default: return 9;
            }
        }
        static int DenseMin(int value) {
            switch (value) {
                case -2147483648: return 1;
                case -2147483647: return 2;
                case -2147483646: return 3;
                case -2147483645: return 4;
                default: return 9;
            }
        }
        static int NestedSwitchReturn(int value) {
            switch (value) {
                case 1:
                    switch (value) { case 1: break; default: break; }
                    return 7;
                default: return 9;
            }
        }
        public static int IntegerCases() { return Select(-1) * 1000 + Select(0) * 100 + Select(1) * 10 + Select(20); }
        public static int StringCases() {
            string prefix = "al";
            return SelectString(prefix + "pha") * 1000 + SelectString("beta") * 100 + SelectString(null) * 10 + SelectString("z");
        }
        public static int EscapedStringCases() { return SelectEscaped("a") * 100000 + SelectEscaped("Л") * 10000 + SelectEscaped("😀") * 1000 + SelectEscaped("🙂") * 100 + SelectEscaped("𐀀") * 10 + SelectEscaped("\x7\x8\xC\xB"); }
        public static int EscapedExpression() { if ("\u0061" == "a" && "\x41B" == "Л" && "\uD83D\uDE00" == "😀") return 1; return 0; }
        public static int BooleanCases() { return SelectBool(true) * 10 + SelectBool(false); }
        public static int RadixCases() { return HexAndBinary(30) * 100 + HexAndBinary(255) * 10 + HexAndBinary(10); }
        public static int DenseCases() { return Dense(-3) * 10000000 + Dense(-2) * 1000000 + Dense(-1) * 100000 + Dense(0) * 10000 + Dense(1) * 1000 + Dense(2) * 100 + Dense(3) * 10 + Dense(4); }
        public static int DenseExtremeCases() { return DenseLow(-2147483647 - 1) * 1000 + DenseLow(-2147483647) * 100 + DenseLow(-2147483644) * 10 + DenseLow(2147483647); }
        public static int DenseMinCases() { return DenseMin(-2147483648) * 1000 + DenseMin(-2147483647) * 100 + DenseMin(-2147483645) * 10 + DenseMin(2147483647); }
        public static int DenseWithFilter() {
            state = 0;
            try {
                switch (0) {
                    case -2: state = 1; break;
                    case -1: state = 2; break;
                    case 0: throw new ArgumentException();
                    case 1: state = 4; break;
                    default: state = 9; break;
                }
            } catch (ArgumentException error) when (error != null) { state = 5; }
            return state;
        }
        public static int NestedSwitchCases() { return NestedSwitchReturn(1) * 10 + NestedSwitchReturn(2); }
        public static int ConstantExpression() {
            switch (2) { case 1 + 1: return 23; default: return -1; }
        }
        public static int EmptySwitchEvaluatesOnce() { state = 0; switch (Probe()) {} return state; }
        public static int LoopBreakAndContinue() {
            int total = 0;
            for (int i = 0; i < 3; i++) {
                switch (i) { case 0: total += 1; break; case 1: continue; default: total += 100; break; }
                total += 10;
            }
            return total;
        }
        public static int NestedLoopBreak() {
            int total = 0;
            switch (2) {
                case 2: while (true) { total += 1; break; } total += 10; break;
                default: total = 99; break;
            }
            return total;
        }
        public static int NestedSwitchContinue() {
            int total = 0;
            for (int i = 0; i < 2; i++) {
                switch (i) {
                    case 0: switch (i) { case 0: continue; default: break; } total = 99; break;
                    default: total += 2; break;
                }
                total += 10;
            }
            return total;
        }
        public static int BreakThroughFinally() {
            int total = 0;
            switch (1) {
                case 1: try { total = 1; break; } finally { total = total * 10 + 2; }
                default: total = 99; break;
            }
            return total;
        }
        public static int SwitchInsideFinally() {
            int total = 0;
            try { total = 1; }
            finally { switch (total) { case 1: total = 3; break; default: break; } }
            return total;
        }
        public static int ContinueThroughFinally() {
            int total = 0;
            for (int i = 0; i < 2; i++) {
                try { switch (i) { case 0: continue; default: total += 2; break; } }
                finally { total = total * 10 + 1; }
            }
            return total;
        }
        public static int BlockScopes() {
            int total = 0;
            switch (1) {
                case 1: { int local = 3; total += local; break; }
                default: { int local = 7; total += local; break; }
            }
            return total;
        }
        public static int NestedDo() {
            int total = 0;
            switch (1) { case 1: do { total += 1; } while (total < 2); break; default: break; }
            return total;
        }
    }
}
