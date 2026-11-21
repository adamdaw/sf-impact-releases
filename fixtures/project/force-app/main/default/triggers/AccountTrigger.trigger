trigger AccountTrigger on Account (before insert, before update) {
  new AccountRepository().rename(Trigger.new[0], 'renamed');
}
