#if LOCAL_FLAG
#error A source-local symbol must not leak between files
#endif
#undef FLAG
#if FLAG
#error Undefinition must apply to this file
#endif
namespace Demo { public class Other { public static int Get() { return 3; } } }
