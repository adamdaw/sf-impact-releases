public abstract class BaseShape implements Shape {
  protected String label;

  public virtual String describe(String prefix) {
    return prefix + label;
  }

  public abstract Decimal area();
}
