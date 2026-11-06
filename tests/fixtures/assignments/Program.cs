namespace AssignmentFixture {
    public class Box {
        public int Value;
        public string Text;
        public int Property { get; set; }
        public float Number { get; set; }
        public static int Shared { get; set; }
        int backing;
        int calls;
        public int Twice { get { return backing; } set { backing = value * 2; } }
        public int Observed { get { calls = calls * 10 + 1; return backing; } set { calls = calls * 10 + 2; backing = value; } }
        public int Prefix() { Value = 3; int result = ++Value; return result * 10 + Value; }
        public int Postfix() { Value = 3; int result = Value++; return result * 10 + Value; }
        public int AssignmentExpression() { int result = (Value = 7); return result * 10 + Value; }
        public string StringAssignment() { string result = (Text = "x"); return result + Text; }
        public float FloatAssignment() { int value = 3; float result = (Number = value); return result + Number; }
        public int ShadowLocal() { this.Value = 1; int Value = 5; Value += 2; return Value * 10 + this.Value; }
        public int ShadowParameter(int Value) { Value += 2; return Value * 10 + this.Value; }
        public int PropertyPrefix() { Property = 3; int result = ++Property; return result * 10 + Property; }
        public int PropertyPostfix() { Property = 3; int result = Property++; return result * 10 + Property; }
        public int PropertyAssignment() { int result = (Twice = 7); return result * 100 + Twice; }
        public int PropertyShadow() { this.Property = 1; int Property = 5; Property += 2; return Property * 10 + this.Property; }
        public int PropertyCompound() { Property = 3; Property += 4; return Property; }
        public int PropertyEffects() { backing = 4; calls = 0; int result = Observed++; return calls * 100 + result * 10 + backing; }
        public int AssignmentEffects() { backing = 0; calls = 0; int result = (Observed = 5); return calls * 100 + result * 10 + backing; }
        public int LocalIncrement() { int value = 3; int first = value++; int second = ++value; return first * 100 + second * 10 + value; }
        public int ParameterIncrement(int value) { int first = value++; int second = ++value; return first * 100 + second * 10 + value; }
        public static int StaticProperty() { Shared = 3; int result = Shared++; Shared += 4; return result * 10 + Shared; }
    }
    public static class Runner {
        public static int Prefix() { Box b = new Box(); return b.Prefix(); }
        public static int Postfix() { Box b = new Box(); return b.Postfix(); }
        public static int AssignmentExpression() { Box b = new Box(); return b.AssignmentExpression(); }
        public static string StringAssignment() { Box b = new Box(); return b.StringAssignment(); }
        public static float FloatAssignment() { Box b = new Box(); return b.FloatAssignment(); }
        public static int ShadowLocal() { Box b = new Box(); return b.ShadowLocal(); }
        public static int ShadowParameter() { Box b = new Box(); return b.ShadowParameter(5); }
        public static int PropertyPrefix() { Box b = new Box(); return b.PropertyPrefix(); }
        public static int PropertyPostfix() { Box b = new Box(); return b.PropertyPostfix(); }
        public static int PropertyAssignment() { Box b = new Box(); return b.PropertyAssignment(); }
        public static int PropertyShadow() { Box b = new Box(); return b.PropertyShadow(); }
        public static int PropertyCompound() { Box b = new Box(); return b.PropertyCompound(); }
        public static int PropertyEffects() { Box b = new Box(); return b.PropertyEffects(); }
        public static int AssignmentEffects() { Box b = new Box(); return b.AssignmentEffects(); }
        public static int LocalIncrement() { Box b = new Box(); return b.LocalIncrement(); }
        public static int ParameterIncrement() { Box b = new Box(); return b.ParameterIncrement(3); }
        public static int StaticProperty() => Box.StaticProperty();
    }
}
