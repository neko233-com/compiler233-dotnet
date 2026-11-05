namespace Demo
{
    public static class GenericRunner
    {
        public static int Run()
        {
            return Demo.Generic.Pairs.Identity<int>(7);
        }
    }
}
