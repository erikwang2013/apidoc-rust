namespace java com.example.demo

include "shared.thrift"

typedef i64 UserId

enum Role {
  ADMIN = 1,
  USER = 2,
}

struct Address {
  1: string street,
  2: optional string city,
}

struct User {
  1: required UserId id,
  2: string name,
  3: optional Address address,
  4: list<string> tags,
  5: map<string, string> extra,
}

service UserService {
  User getUser(1: UserId id) throws (1: string not_found),
  void ping(),
  oneway void log(1: string message),
}
