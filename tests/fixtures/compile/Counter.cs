namespace Demo
{
    public class Counter
    {
        private int value = 0;

        public Counter(int start)
        {
            value = start;
        }

        public int Add(int amount)
        {
            value = value + amount;
            return value;
        }

        public int Value()
        {
            return value;
        }

        public int Bonus { get; set; }

        public int Doubled()
        {
            return this.Value() + Value();
        }
    }
}
