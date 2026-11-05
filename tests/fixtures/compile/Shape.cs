namespace Demo
{
    public interface INamed
    {
        int Score();
    }

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

    public class Square : Shape, INamed
    {
        public override int Area()
        {
            return 4;
        }

        public int Score()
        {
            return 2;
        }
    }
}
