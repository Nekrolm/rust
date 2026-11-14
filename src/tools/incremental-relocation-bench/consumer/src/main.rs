use bevy_ecs::prelude::*;
#[derive(Component)] struct Position<T: Send + Sync + 'static>(T);
#[derive(Component)] struct Velocity<T: Send + Sync + 'static>(T);
fn exercise<T: Default + Copy + Send + Sync + 'static>(world: &mut World) -> usize {
    world.spawn((Position(T::default()), Velocity(T::default())));
    let mut state = world.query::<(&Position<T>, &Velocity<T>)>();
    state.iter(world).count()
}
fn main() {
    let mut world = World::new();
    let count = exercise::<u8>(&mut world) + exercise::<u16>(&mut world)
        + exercise::<u32>(&mut world) + exercise::<u64>(&mut world)
        + exercise::<i8>(&mut world) + exercise::<i16>(&mut world)
        + exercise::<i32>(&mut world) + exercise::<i64>(&mut world)
        + exercise::<f32>(&mut world) + exercise::<f64>(&mut world);
    println!("{count}");
}
