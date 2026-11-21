public inherited sharing class AccountRepository {
  public List<Account> withContacts(Set<Id> ids, String city) {
    return [
      SELECT Id, Name, Owner.Email, (SELECT LastName FROM Contacts)
      FROM Account a
      WHERE a.BillingCity = :normalise(city) AND Id IN :ids
      ORDER BY Name
    ];
  }

  public void rename(Account account, String name) {
    account.Name = name;
    update account;
  }

  public void save(List<Invoice__c> invoices) {
    upsert invoices External_Id__c;
    insert new npsp__Gift__c(npsp__Amount__c = 10);
    delete [SELECT Id FROM Invoice__c WHERE Paid__c = TRUE];
  }

  public void archive(List<Invoice__c> invoices) {
    Database.delete(invoices, false);
  }

  public Id testClassId() {
    // Built at run time: a string, so only the textual tier sees the name in it.
    String soql = 'SELECT Id FROM ApexClass WHERE Name = \'ShapeServiceTest\'';
    return Database.query(soql)[0].Id;
  }

  private String normalise(String city) {
    return city.trim();
  }
}
