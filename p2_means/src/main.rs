struct I32 {
    val: i32
}
struct InsertMessage {
    timestamp: I32,
    price: I32
}
struct QueryMessage {
    mintime: I32,
    maxtime: I32
}

enum Message {
    Insert(InsertMessage),
    Query(QueryMessage)
}

// impl 


fn main() {

}