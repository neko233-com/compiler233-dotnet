namespace Demo
{
    public class Shape
    {
        public virtual int Area()
        {
            return 0;
        }

        public int Tag()
        {
            return 3;
        }
    }

    public class Square : Shape
    {
        public override int Area()
        {
            return 4;
        }
    }
}
