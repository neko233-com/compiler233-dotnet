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
            total = total + Tally.Add(2);
            total = total + Tally.Add(3);

            int skipped = 0;
            for (i = 0; i < 6; i++)
            {
                if (i == 2) { continue; }
                if (i == 5) { break; }
                skipped = skipped + i;
            }
            total = total + skipped;

            Counter first = new Counter(10);
            Counter second = new Counter(1);
            total = total + first.Add(2) + second.Add(3);
            total = total + first.Doubled();

            string label = "compiler233";
            if (label == "compiler233" && label != "roslyn")
            {
                total = total + 1;
            }
            first.Bonus = 5;
            total = total + first.Bonus;

            int[] values = new int[3];
            values[0] = 1;
            values[1] = 2;
            values[2] = 3;
            int sum = 0;
            foreach (int value in values)
            {
                sum = sum + value;
            }
            total = total + sum + values.Length;
            return total;
        }
    }
}
