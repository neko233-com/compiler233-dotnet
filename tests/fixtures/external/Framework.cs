namespace Demo
{
    public static class FrameworkRunner
    {
        public static int Run()
        {
            if (System.Text.RegularExpressions.Regex.IsMatch("abc123", "\\d+"))
            {
                return 1;
            }
            return 0;
        }
    }
}
