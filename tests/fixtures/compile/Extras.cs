namespace Demo
{
    public enum Tier
    {
        Basic = 1,
        Pro
    }

    public class Extras
    {
        public float weight = 0.5f;
        public string note;
        public int Total { get; set; }

        public int Bump(int amount)
        {
            Total += amount;
            return Total;
        }

        public float Scaled(float factor)
        {
            return weight * factor;
        }

        public string Note(string fallback)
        {
            return note ?? fallback;
        }
    }
}
