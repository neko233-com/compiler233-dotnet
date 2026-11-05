namespace Demo
{
    public class Animal
    {
        public virtual int Legs()
        {
            return 2;
        }
    }

    public class Dog : Animal
    {
        public override int Legs()
        {
            return base.Legs() + 2;
        }
    }

    public static class BaseRunner
    {
        public static int Run()
        {
            Dog dog = new Dog();
            Animal animal = dog;
            return animal.Legs();
        }
    }
}
