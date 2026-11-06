#if FLAG
#define LOCAL_FLAG
#endif

namespace Demo {
    public class Program {
        public static int Run() {
#if FLAG && SECOND
            return 11;
#elif FLAG
  #if LOCAL_FLAG
            return 29;
  #else
            return 99;
  #endif
#else
            return 7;
#endif
        }
    }
}
