namespace Demo
{
    public static class Program
    {
        public static int Run()
        {
            int total = 0;
            int i = 1;
            while (i <= 10)
            {
                total = total + MathUtil.Fibonacci(i) + MathUtil.SumTo(i);
                i = i + 1;
            }
            if (total > 0 && total < 1000000)
            {
                total = total + 1;
            }
            total = total + Counter.Add(2);
            total = total + Counter.Add(3);
            return total;
        }
    }
}
