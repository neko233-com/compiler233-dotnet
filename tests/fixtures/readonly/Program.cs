namespace ReadonlyFixture {
    public enum Choice { One = 1, Two = 2 }
    public class Holder {
        public readonly int A = 2, B = 3;
        public readonly int C;
        public readonly int[] Numbers = new int[1];
        public readonly Choice Pick;
        public static readonly int Global = 3;
        public static readonly int Inline;
        public static int Trigger = (Inline = 7);
        static Holder() { Global += 2; Holder.Global++; Inline += 1; }
        public Holder(int value) {
            C = value;
            this.A += 1;
            this.C++;
            Numbers[0] = value;
            Pick = Choice.Two;
        }
        public int Sum() { return A + B + C + Numbers[0] + (int)Pick + Global + Inline + Trigger; }
        public static int Run() {
            Holder value = new Holder(4);
            value.Numbers[0] = 5;
            return value.Sum();
        }
    }
}
