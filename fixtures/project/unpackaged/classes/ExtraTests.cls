@IsTest
private class ExtraTests {
  @IsTest
  static void billingRuns() {
    Assert.isNotNull(new BillingPlugin().run(new Invoice()));
  }
}
