namespace Demo
{
    public static class Counter
    {
        private static int value = 0;
        public static int Add(int amount)
        {
            value = value + amount;
            return value;
        }
    }
}
