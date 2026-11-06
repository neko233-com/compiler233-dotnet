using System;
namespace ExceptionFixture {
    public class CustomException : Exception { public CustomException() : base() {} }
    public static class Runner {
        static int state;
        static int Divide(int value) => 10 / value;
        public static int Catch() {
            int result = 0;
            try { result = Divide(0); }
            catch (DivideByZeroException error) { result = 7; }
            return result;
        }
        public static int Multiple() {
            int result = 0;
            try { throw new ArgumentNullException("key"); }
            catch (DivideByZeroException) { result = 1; }
            catch (ArgumentException) { result = 2; }
            return result;
        }
        public static int GeneralCatch() {
            int result = 0;
            try { throw null; } catch { result = 3; }
            return result;
        }
        static int ReturnCore() {
            try { return 5; } finally { state = 7; }
        }
        public static int ReturnFinally() { state = 0; int result = ReturnCore(); return result * 10 + state; }
        static void ThrowCore() {
            try { throw new ArgumentException("bad"); }
            catch (ArgumentException) { state = 1; throw; }
            finally { state += 2; }
        }
        public static int RethrowFinally() {
            state = 0;
            try { ThrowCore(); } catch (ArgumentException) { return state; }
            return -1;
        }
        public static int LoopLeave() {
            int i = 0; int sum = 0;
            while (i < 4) {
                i += 1;
                try { if (i == 2) continue; if (i == 3) break; sum += i; }
                finally { sum += 10; }
            }
            return sum * 10 + i;
        }
        public static int LoopInsideTry() {
            int i = 0; int sum = 0;
            try { while (i < 4) { i += 1; if (i == 2) continue; if (i == 3) break; sum += i; } }
            finally { sum += 10; }
            return sum * 10 + i;
        }
        public static int Custom() {
            int result = 0;
            try { throw new CustomException(); } catch (CustomException) { result = 9; }
            return result;
        }
        public static int CatchVariable() {
            int result = 0;
            try { try { throw new ArgumentException(); } catch (ArgumentException error) { throw error; } }
            catch (ArgumentException) { result = 11; }
            return result;
        }
        public static int FinallyOnUnhandled() {
            state = 0;
            try {
                try { throw new InvalidOperationException(); }
                catch (DivideByZeroException) { state = 1; }
                finally { state = 5; }
            } catch (InvalidOperationException) { state += 2; }
            return state;
        }
        public static int AllPathsReturn() {
            try { if (state == 0) return 3; throw new ArgumentException(); }
            catch (ArgumentException) { return 4; }
        }
        public static int FinallyThrows() {
            try { try { return 2; } finally { throw new ArgumentException(); } }
            catch (ArgumentException) { return 8; }
        }
        static void VoidCore() { try { return; } finally { state = 6; } }
        public static int VoidFinally() { state = 0; VoidCore(); return state; }
        public static string StringFinally() { try { return "kept"; } finally { state = 9; } }
        public static bool BoolFinally() { try { return true; } finally { state = 9; } }
        public static int NestedRethrow() {
            state = 0;
            try {
                try { throw new ArgumentException(); }
                catch (ArgumentException) { try { throw; } finally { state += 1; } }
            } catch (ArgumentException) { return state; }
        }
    }
}
