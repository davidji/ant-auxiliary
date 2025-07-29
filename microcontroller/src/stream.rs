
pub trait Supplier {
    type Item;

    async fn next(&mut self) -> Option<Self::Item>;
}

pub trait ByteSupplier: Supplier<Item = u8> {}

pub trait Consumer {
    type Item;
    type Error;

    async fn accept(&mut self, item: Self::Item) -> Result<(), Self::Error>;
}

pub async fn relay<I, O, T, E>(input: &mut I,  output: &mut O) -> Result<(), E>
where
    I: Supplier<Item = T>,
    O: Consumer<Item = T, Error = E>,
{
    while let Some(item) = input.next().await {
        output.accept(item).await?;
    }
    Ok(())
}

pub trait ByteConsumer: Consumer<Item = u8> {}
