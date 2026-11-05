namespace Demo
{
    public static class MathUtil
    {
        public static int Fibonacci(int n)
        {
            if (n < 2)
            {
                return n;
            }
            int a = 0;
            int b = 1;
            int i = 2;
            for (i = 2; i <= n; i = i + 1)
            {
                int next = a + b;
                a = b;
                b = next;
            }
            return b;
        }

        public static int SumTo(int n)
        {
            int total = 0;
            int i = 1;
            while (i <= n)
            {
                total = total + i;
                i = i + 1;
            }
            return total;
        }
    }
}
