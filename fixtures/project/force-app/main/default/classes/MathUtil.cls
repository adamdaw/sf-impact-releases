public class MathUtil {
  public static final Decimal PI_ISH = 3.14;

  public enum Mode {
    UP,
    DOWN
  }

  public static Decimal round(Decimal value) {
    return round(value, 0);
  }

  public static Decimal round(Decimal value, Integer places) {
    return value.setScale(places);
  }

  public static Integer clamp(Integer value, Integer low, Integer high) {
    return Math.max(low, Math.min(high, value));
  }

  public static Decimal clamp(Decimal value, Decimal low, Decimal high) {
    return Math.max(low, Math.min(high, value));
  }

  public static Integer sign(Mode mode) {
    switch on mode {
      when UP {
        return 1;
      }
      when else {
        return -1;
      }
    }
  }

  public static String describe(Object value) {
    return String.valueOf(value);
  }

  public static String describe(Shape shape) {
    return shape.describe('a ');
  }
}
